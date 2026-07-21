//! Account / control-panel APIs (login, folder/file management).
//! Ported from `lanzou.class.php`. Credentials are never hard-coded.

use crate::error::{Error, Result};
use reqwest::blocking::multipart::{Form, Part};
use reqwest::blocking::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, SET_COOKIE, USER_AGENT};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_ACCOUNT_BASE: &str = "https://pc.woozooo.com/";
const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
/// Lanzou HTML5 upload size limit (server-side).
const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;

/// Server-side suffix whitelist for `html5up.php`.
const UPLOAD_ALLOWED_EXTS: &[&str] = &[
    "doc", "docx", "zip", "rar", "apk", "txt", "exe", "7z", "e", "z", "ct", "ke",
    "cetrainer", "db", "tar", "pdf", "w3x", "epub", "mobi", "azw", "azw3", "osk",
    "osz", "xpa", "cpk", "lua", "jar", "dmg", "ppt", "pptx", "xls", "xlsx", "mp3",
    "ipa", "iso", "img", "gho", "ttf", "ttc", "txf", "dwg", "bat", "imazingapp",
    "dll", "crx", "xapk", "conf", "deb", "rp", "rpm", "rplib", "mobileconfig",
    "appimage", "lolgezi", "flac", "cad", "hwt", "accdb", "ce", "xmind", "enc",
    "bds", "bdi", "ssf", "it", "pkg", "cfg", "mp4", "avi", "png", "jpeg", "jpg",
    "gif", "webp", "brushset",
];

/// Whether `ext` (with or without leading `.`) is accepted by html5up.php.
pub fn is_upload_allowed_ext(ext: &str) -> bool {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    !ext.is_empty() && UPLOAD_ALLOWED_EXTS.iter().any(|e| *e == ext)
}

fn file_ext(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

fn zip_single_file(src: &Path, suffix_name: &str) -> Result<(PathBuf, String)> {
    let base = src
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::Parse("invalid filename".into()))?
        .to_string();
    let suffix = if suffix_name.is_empty() {
        "zip"
    } else {
        suffix_name
    };
    let upload_name = format!("{base}.{suffix}");
    let tmp = std::env::temp_dir().join(format!(
        "lanzou-up-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let file = File::create(&tmp)?;
    let mut zip = zip::ZipWriter::new(file);
    let opts: zip::write::FileOptions<'_, ()> =
        zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    zip.start_file(&base, opts)
        .map_err(|e| Error::Parse(format!("zip start: {e}")))?;
    let mut src_f = File::open(src)?;
    let mut buf = Vec::new();
    src_f.read_to_end(&mut buf)?;
    zip.write_all(&buf)
        .map_err(|e| Error::Parse(format!("zip write: {e}")))?;
    zip.finish()
        .map_err(|e| Error::Parse(format!("zip finish: {e}")))?;
    Ok((tmp, upload_name))
}

fn rename_copy(src: &Path) -> Result<PathBuf> {
    let tmp = std::env::temp_dir().join(format!(
        "lanzou-up-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    fs::copy(src, &tmp)?;
    Ok(tmp)
}

fn is_archive_suffix(ext: &str) -> bool {
    matches!(
        ext.trim_start_matches('.').to_ascii_lowercase().as_str(),
        "zip" | "rar" | "7z" | "tar"
    )
}

/// Apply suffix_auto_convert policy for a single ≤100MB file.
fn convert_suffix(local_path: &Path, cfg: &crate::config::Config) -> Result<(PathBuf, String, Option<PathBuf>)> {
    let name = local_path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| Error::Parse("invalid filename".into()))?
        .to_string();
    if is_upload_allowed_ext(&file_ext(&name)) {
        return Ok((local_path.to_path_buf(), name, None));
    }
    if !cfg.suffix_auto_convert {
        return Err(Error::Parse(format!(
            "suffix .{} not allowed by server (set suffix_auto_convert=true or rename)",
            file_ext(&name)
        )));
    }
    let suffix = if cfg.suffix_name.is_empty() {
        "zip".into()
    } else {
        cfg.suffix_name.clone()
    };
    if !is_upload_allowed_ext(&suffix) {
        return Err(Error::Parse(format!(
            "configured suffix_name .{suffix} is not on server whitelist"
        )));
    }
    if cfg.suffix_mode == "rename" {
        let upload_name = format!("{name}.{suffix}");
        let p = rename_copy(local_path)?;
        Ok((p.clone(), upload_name, Some(p)))
    } else {
        let (zp, zn) = zip_single_file(local_path, &suffix)?;
        Ok((zp.clone(), zn, Some(zp)))
    }
}

fn split_file(local_path: &Path, chunk_bytes: u64) -> Result<(Vec<PathBuf>, Vec<u64>)> {
    let mut f = File::open(local_path)?;
    let mut paths = Vec::new();
    let mut sizes = Vec::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut idx = 0usize;
    loop {
        let tmp = std::env::temp_dir().join(format!(
            "lanzou-part-{}-{}-{}",
            std::process::id(),
            idx,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let mut out = File::create(&tmp)?;
        let mut written: u64 = 0;
        while written < chunk_bytes {
            let to_read = std::cmp::min(buf.len() as u64, chunk_bytes - written) as usize;
            let n = f.read(&mut buf[..to_read])?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            written += n as u64;
        }
        drop(out);
        if written == 0 {
            let _ = fs::remove_file(&tmp);
            break;
        }
        paths.push(tmp);
        sizes.push(written);
        idx += 1;
        if written < chunk_bytes {
            break;
        }
    }
    if paths.is_empty() {
        return Err(Error::Parse("empty file, nothing to split".into()));
    }
    Ok((paths, sizes))
}

/// Entry type: folder or file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    Folder = 0,
    File = 1,
}

/// Folder or file list row.
#[derive(Debug, Clone)]
pub struct ListEntry {
    pub kind: EntryKind,
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub size: Option<String>,
    pub time: Option<String>,
    pub description: Option<String>,
}

/// Scraped folder metadata.
#[derive(Debug, Clone, Default)]
pub struct FolderInfo {
    pub name: String,
    pub description: String,
    pub url: String,
    pub password: String,
    pub file_count: String,
    pub file_size: String,
}

/// Parsed managed-file info (task=22).
#[derive(Debug, Clone)]
pub struct FileInfo {
    pub id: String,
    pub password: String,
    pub share_url: String,
    pub raw: Value,
}

/// Logged-in Lanzou control-panel client.
pub struct Account {
    http: HttpClient,
    base: String,
    cookie: String,
    cookie_file: Option<PathBuf>,
    username: String,
    password: String,
}

impl Account {
    pub fn new(username: impl Into<String>, password: impl Into<String>) -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_UA));
        headers.insert(
            reqwest::header::ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );
        let http = HttpClient::builder()
            .default_headers(headers)
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self {
            http,
            base: DEFAULT_ACCOUNT_BASE.into(),
            cookie: String::new(),
            cookie_file: None,
            username: username.into(),
            password: password.into(),
        })
    }

    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        let mut b = base.into();
        if !b.ends_with('/') {
            b.push('/');
        }
        self.base = b;
        self
    }

    pub fn with_cookie_file(mut self, path: impl AsRef<Path>) -> Self {
        let p = path.as_ref().to_path_buf();
        if let Ok(s) = fs::read_to_string(&p) {
            self.cookie = s.trim().to_string();
        }
        self.cookie_file = Some(p);
        self
    }

    pub fn cookie(&self) -> &str {
        &self.cookie
    }

    pub fn set_cookie(&mut self, cookie: impl Into<String>) -> Result<()> {
        self.cookie = cookie.into();
        if let Some(p) = &self.cookie_file {
            fs::write(p, &self.cookie)?;
        }
        Ok(())
    }

    fn store_set_cookie(&mut self, headers: &HeaderMap) {
        let mut parts: Vec<String> = Vec::new();
        // keep existing non-overwritten pairs
        let mut map: HashMap<String, String> = HashMap::new();
        for pair in self.cookie.split(';').filter(|s| !s.trim().is_empty()) {
            if let Some((k, v)) = pair.split_once('=') {
                map.insert(k.trim().to_string(), v.trim().to_string());
            }
        }
        for val in headers.get_all(SET_COOKIE) {
            if let Ok(s) = val.to_str() {
                if let Some(pair) = s.split(';').next() {
                    if let Some((k, v)) = pair.split_once('=') {
                        map.insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
            }
        }
        for (k, v) in map {
            parts.push(format!("{k}={v}"));
        }
        self.cookie = parts.join("; ");
    }

    /// Login with simple POST to mlogin.php (task/uid/pwd).
    pub fn login(&mut self) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}mlogin.php", self.base))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(USER_AGENT, DEFAULT_UA)
            .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9")
            .header(reqwest::header::REFERER, format!("{}mlogin.php", self.base))
            .form(&[
                ("task", "3"),
                ("uid", self.username.as_str()),
                ("pwd", self.password.as_str()),
            ])
            .send()?;
        self.store_set_cookie(resp.headers());
        let status = resp.status();
        let raw = resp.text()?;
        if !status.is_success() {
            return Err(Error::Http(format!("login status {status} body={raw}")));
        }
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Parse(format!("login non-json: {e}; {raw}")))?;
        if json_str(&v["zt"]) != "1" {
            let info = json_str(&v["info"]);
            let msg = if info.is_empty() { raw } else { info };
            return Err(Error::Parse(format!("login failed: {msg}")));
        }
        if self.cookie.is_empty() {
            return Err(Error::Parse("login ok but no Set-Cookie received".into()));
        }
        if let Some(p) = &self.cookie_file {
            fs::write(p, &self.cookie)?;
        }
        Ok(())
    }


    /// True when session cookie is still valid.

    pub fn verification(&self) -> bool {
        if self.cookie.is_empty() {
            return false;
        }
        if let Ok(raw) = self.post_task("task=5&folder_id=-1") {
            if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                let zt = json_str(&v["zt"]);
                if !zt.is_empty() && zt != "9" {
                    return true;
                }
            }
        }
        match self.get_html("account.php") {
            Ok(html) => !html.contains("网盘用户登录"),
            Err(_) => false,
        }
    }

    pub fn ensure_login(&mut self) -> Result<()> {
        if self.verification() {
            return Ok(());
        }
        self.login()
    }

    fn post_task(&self, param: &str) -> Result<String> {
        let mut req = self
            .http
            .post(format!("{}doupload.php", self.base))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(param.to_string());
        if !self.cookie.is_empty() {
            req = req.header(COOKIE, &self.cookie);
        }
        let resp = req.send()?;
        let status = resp.status();
        let raw = resp.text()?;
        if !status.is_success() {
            return Err(Error::Http(format!(
                "doupload {status}: {}",
                &raw[..raw.len().min(200)]
            )));
        }
        Ok(raw)
    }

    fn get_html(&self, path_or_url: &str) -> Result<String> {
        let url = if path_or_url.starts_with("http") {
            path_or_url.to_string()
        } else {
            format!(
                "{}{}",
                self.base,
                path_or_url.trim_start_matches('/')
            )
        };
        let mut req = self.http.get(&url);
        if !self.cookie.is_empty() {
            req = req.header(COOKIE, &self.cookie);
        }
        let resp = req.send()?;
        let status = resp.status();
        let raw = resp.text()?;
        if !status.is_success() {
            return Err(Error::Http(format!("GET {url} -> {status}")));
        }
        Ok(raw)
    }

    /// List folders + files under `folder_id` (`"-1"` = root).
    /// Folders via task=47, files via task=5.
    pub fn list(&self, folder_id: &str) -> Result<Vec<ListEntry>> {
        let folder_id = if folder_id.is_empty() { "-1" } else { folder_id };
        let mut out = Vec::new();

        let raw_dir = self.post_task(&format!("task=47&folder_id={folder_id}"))?;
        let vdir: Value = serde_json::from_str(&raw_dir)
            .map_err(|e| Error::Parse(format!("list folders json: {e}; {raw_dir}")))?;
        if let Some(arr) = vdir.get("text").and_then(|t| t.as_array()) {
            for it in arr {
                let id = json_str(&it["fol_id"]);
                if id.is_empty() {
                    continue;
                }
                out.push(ListEntry {
                    kind: EntryKind::Folder,
                    id,
                    name: json_str(&it["name"]),
                    url: None,
                    size: None,
                    time: None,
                    description: Some(json_str(&it["folder_des"])).filter(|s| !s.is_empty()),
                });
            }
        }

        let raw = self.post_task(&format!("task=5&folder_id={folder_id}"))?;
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Parse(format!("list files json: {e}; {raw}")))?;
        if let Some(arr) = v.get("text").and_then(|t| t.as_array()) {
            for it in arr {
                let id = json_str(&it["id"]);
                out.push(ListEntry {
                    kind: EntryKind::File,
                    id,
                    name: json_str(&it["name_all"]),
                    url: None,
                    size: Some(json_str(&it["size"])).filter(|s| !s.is_empty()),
                    time: Some(json_str(&it["time"])).filter(|s| !s.is_empty()),
                    description: None,
                });
            }
        }
        Ok(out)
    }

    pub fn get_folder_id_by_name(&self, name: &str, father_id: &str) -> Result<String> {
        for e in self.list(father_id)? {
            if e.kind == EntryKind::Folder && e.name == name {
                return Ok(e.id);
            }
        }
        Err(Error::Parse(format!("folder {name:?} not found")))
    }

    pub fn create_folder(&self, name: &str, parent_id: &str, describe: &str) -> Result<String> {
        let parent_id = if parent_id.is_empty() {
            "-1"
        } else {
            parent_id
        };
        if let Ok(id) = self.get_folder_id_by_name(name, parent_id) {
            return Err(Error::Parse(format!("folder already exists: id={id}")));
        }
        self.post_task(&format!(
            "task=2&folder_name={}&folder_description={}&parent_id={parent_id}",
            urlencoding::encode(name),
            urlencoding::encode(describe),
        ))
    }

    pub fn set_folder_name_and_describe(
        &self,
        folder_id: &str,
        name: &str,
        describe: &str,
    ) -> Result<String> {
        self.post_task(&format!(
            "task=4&folder_id={folder_id}&folder_name={}&folder_description={}",
            urlencoding::encode(name),
            urlencoding::encode(describe),
        ))
    }

    pub fn set_folder_password(&self, folder_id: &str, pwd: &str) -> Result<String> {
        self.post_task(&format!(
            "task=16&shows=1&shownames={}&folder_id={folder_id}",
            urlencoding::encode(pwd),
        ))
    }

    pub fn delete_folder(&self, folder_id: &str) -> Result<String> {
        self.post_task(&format!("task=3&folder_id={folder_id}"))
    }

    pub fn delete_folder_by_name(&self, name: &str, father_id: &str) -> Result<String> {
        let id = self.get_folder_id_by_name(name, father_id)?;
        self.delete_folder(&id)
    }

    pub fn get_folder_info(&self, folder_id: &str) -> Result<FolderInfo> {
        let html = self.get_html(&format!(
            "myfile.php?item=3&folder_id={folder_id}&v2"
        ))?;
        Ok(FolderInfo {
            name: str_intercept(
                &html,
                r#"<input class="input" type="text" id="foldertxt" name="foldertxt" value=""#,
                r#"">"#,
            ),
            description: str_intercept(
                &html,
                r#"<input class="input" type="text" id="folderinfo" name="folderinfo" value=""#,
                r#"">"#,
            ),
            url: str_intercept(
                &html,
                &format!(r#"<div class="folsha8"><div class="f_pwdurl" onclick="ucopy({folder_id});">"#),
                "<br>",
            ),
            password: str_intercept(&html, r#"<span class="shapwd">密码:"#, r#"</span></div>"#),
            file_count: str_intercept(
                &html,
                r#"<div class="folsha2">文件数<div class="folsha3">"#,
                r#"</div></div>"#,
            ),
            file_size: str_intercept(
                &html,
                r#"<div class="folsha2">大小<div class="folsha3">"#,
                r#"</div></div>"#,
            ),
        })
    }

    pub fn get_file_info_raw(&self, file_id: &str) -> Result<String> {
        self.post_task(&format!("task=22&file_id={file_id}"))
    }

    pub fn get_file_info(&self, file_id: &str) -> Result<FileInfo> {
        let raw = self.get_file_info_raw(file_id)?;
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Parse(format!("file info json: {e}; {raw}")))?;
        let info = &v["info"];
        let pwd = json_str(&info["pwd"]);
        let newd = json_str(&info["is_newd"]);
        let fid = json_str(&info["f_id"]);
        let share_url = if !newd.is_empty() && !fid.is_empty() {
            format!("{}/{fid}", newd.trim_end_matches('/'))
        } else {
            String::new()
        };
        Ok(FileInfo {
            id: file_id.into(),
            password: pwd,
            share_url,
            raw: v,
        })
    }

    pub fn get_file_password(&self, file_id: &str) -> Result<String> {
        Ok(self.get_file_info(file_id)?.password)
    }

    pub fn get_file_download_info(&self, file_id: &str) -> Result<(String, String)> {
        let fi = self.get_file_info(file_id)?;
        Ok((fi.share_url, fi.password))
    }

    pub fn set_file_password(&self, file_id: &str, pwd: &str) -> Result<String> {
        self.post_task(&format!(
            "task=23&file_id={file_id}&shows=1&shownames={}",
            urlencoding::encode(pwd),
        ))
    }

    pub fn get_file_describe(&self, file_id: &str) -> Result<String> {
        let raw = self.post_task(&format!("task=12&file_id={file_id}"))?;
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Parse(format!("describe json: {e}; {raw}")))?;
        Ok(json_str(&v["info"]))
    }

    pub fn set_file_describe(&self, file_id: &str, describe: &str) -> Result<String> {
        self.post_task(&format!(
            "task=11&file_id={file_id}&desc={}",
            urlencoding::encode(describe),
        ))
    }

    pub fn move_file(&self, file_id: &str, folder_id: &str) -> Result<String> {
        self.post_task(&format!(
            "task=20&folder_id={folder_id}&file_id={file_id}"
        ))
    }

    pub fn delete_file(&self, file_id: &str) -> Result<String> {
        self.post_task(&format!("task=6&file_id={file_id}"))
    }

    /// Upload a local file via `html5up.php`.
    ///
    /// Behaviour is controlled by [`crate::config::get_config`]:
    /// suffix conversion, large-file split, and part notes.
    pub fn upload(&self, local_path: impl AsRef<Path>, folder_id: &str) -> Result<UploadResult> {
        let local_path = local_path.as_ref();
        let folder_id = if folder_id.is_empty() { "-1" } else { folder_id };
        let cfg = crate::config::get_config();
        let meta = fs::metadata(local_path)?;
        if !meta.is_file() {
            return Err(Error::Parse(format!("not a file: {}", local_path.display())));
        }
        let orig_name = local_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Parse("invalid filename".into()))?
            .to_string();
        let size = meta.len();
        let mut chunk_bytes = (cfg.split_size_mb as u64) * 1024 * 1024;
        if chunk_bytes < 1 {
            chunk_bytes = 90 * 1024 * 1024;
        }
        if chunk_bytes > MAX_UPLOAD_BYTES {
            chunk_bytes = MAX_UPLOAD_BYTES;
        }

        let mut need_split = cfg.split_enable && size > chunk_bytes;
        if !cfg.split_enable && size > MAX_UPLOAD_BYTES {
            return Err(Error::Parse(format!(
                "file too large: {size} bytes (max {} MB; enable split_enable or shrink file)",
                MAX_UPLOAD_BYTES / (1024 * 1024)
            )));
        }

        if !need_split {
            let (up_path, up_name, temp) = convert_suffix(local_path, &cfg)?;
            let _guard = TempGuard(temp.clone());
            let ust = fs::metadata(&up_path)?.len();
            if ust > MAX_UPLOAD_BYTES {
                if !cfg.split_enable {
                    return Err(Error::Parse(format!(
                        "converted file too large: {ust} bytes (max {} MB)",
                        MAX_UPLOAD_BYTES / (1024 * 1024)
                    )));
                }
                need_split = true;
                // drop guard ownership of temp — upload_split will clean via its own temps;
                // keep converted file path for split source
                if need_split {
                    let res = self.upload_split(&up_path, &orig_name, folder_id, &cfg, chunk_bytes);
                    return res;
                }
            }
            let mut res = self.upload_one(&up_path, &up_name, folder_id)?;
            res.orig_name = Some(orig_name.clone());
            // Record original name when suffix was converted.
            if !res.file_id.is_empty() && up_name != orig_name {
                let note = crate::notes::format_convert_note(
                    &orig_name,
                    &up_name,
                    &cfg.suffix_mode,
                    &cfg.suffix_name,
                    size,
                );
                if let Err(e) = self.set_file_describe(&res.file_id, &note) {
                    eprintln!("[warn] set convert note: {e}");
                }
            }
            return Ok(res);
        }

        self.upload_split(local_path, &orig_name, folder_id, &cfg, chunk_bytes)
    }

    fn upload_split(
        &self,
        local_path: &Path,
        orig_name: &str,
        folder_id: &str,
        cfg: &crate::config::Config,
        chunk_bytes: u64,
    ) -> Result<UploadResult> {
        let (paths, sizes) = split_file(local_path, chunk_bytes)?;
        let _paths_guard = MultiTempGuard(paths.clone());
        let total = paths.len();
        let group_id = format!(
            "{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let suffix = if cfg.suffix_name.is_empty() {
            "zip".to_string()
        } else {
            cfg.suffix_name.clone()
        };
        let mut parts = Vec::with_capacity(total);
        let mut first: Option<UploadResult> = None;

        for (i, p) in paths.iter().enumerate() {
            let index = i + 1;
            let mut part_name =
                crate::config::format_split_name(&cfg.split_name_format, orig_name, index, total, &suffix);
            if !is_upload_allowed_ext(&file_ext(&part_name)) {
                part_name = format!("{part_name}.{suffix}");
            }
            let need_real_zip = cfg.suffix_mode == "zip" && is_archive_suffix(&suffix);
            let (up_path, up_name, temp) = if need_real_zip {
                let (zp, _zn) = zip_single_file(p, &suffix)?;
                let mut uname = part_name.clone();
                if file_ext(&uname) != suffix {
                    uname = format!(
                        "{}.{}",
                        Path::new(&uname)
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or(&uname),
                        suffix
                    );
                }
                (zp.clone(), uname, Some(zp))
            } else {
                let mut uname = part_name.clone();
                if !is_upload_allowed_ext(&file_ext(&uname)) {
                    uname = format!("{uname}.{suffix}");
                }
                let cp = rename_copy(p)?;
                (cp.clone(), uname, Some(cp))
            };
            let _g = TempGuard(temp);
            let ust = fs::metadata(&up_path)?.len();
            if ust > MAX_UPLOAD_BYTES {
                return Err(Error::Parse(format!(
                    "part {index} too large after convert: {ust} bytes"
                )));
            }
            let res = self
                .upload_one(&up_path, &up_name, folder_id)
                .map_err(|e| Error::Parse(format!("upload part {index}/{total}: {e}")))?;
            if cfg.split_note && !res.file_id.is_empty() {
                let note = crate::notes::format_part_note(
                    &group_id,
                    orig_name,
                    index,
                    total,
                    sizes[i],
                );
                if let Err(e) = self.set_file_describe(&res.file_id, &note) {
                    eprintln!("[warn] set part note {index}: {e}");
                }
            }
            parts.push(UploadPart {
                file_id: res.file_id.clone(),
                name: res.name.clone(),
                index,
                total,
                size: sizes[i],
            });
            if first.is_none() {
                first = Some(res);
            }
        }
        let mut first = first.ok_or_else(|| Error::Parse("split upload produced no parts".into()))?;
        first.parts = parts;
        first.orig_name = Some(orig_name.to_string());
        first.group_id = Some(group_id);
        Ok(first)
    }

    fn upload_one(&self, local_path: &Path, filename: &str, folder_id: &str) -> Result<UploadResult> {
        let mut urls = vec![format!("{}html5up.php", self.base)];
        if self.base.contains("up.woozooo.com") {
            urls.push("https://pc.woozooo.com/html5up.php".into());
        } else if self.base.contains("pc.woozooo.com") {
            urls.push("https://up.woozooo.com/html5up.php".into());
        }

        let mut last_err = Error::Http("upload failed".into());
        for up_url in urls {
            let mut file = File::open(local_path)?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            let part = Part::bytes(buf)
                .file_name(filename.to_string())
                .mime_str("application/octet-stream")
                .map_err(|e| Error::Http(e.to_string()))?;
            let form = Form::new()
                .text("task", "1")
                .text("folder_id", folder_id.to_string())
                .part("upload_file", part);

            let referer = format!("{}mydisk.php", self.base);
            let mut req = self
                .http
                .post(&up_url)
                .timeout(Duration::from_secs(3600))
                .header(USER_AGENT, DEFAULT_UA)
                .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
                .header(reqwest::header::REFERER, &referer)
                .multipart(form);
            if !self.cookie.is_empty() {
                req = req.header(COOKIE, &self.cookie);
            }
            let resp = match req.send() {
                Ok(r) => r,
                Err(e) => {
                    last_err = Error::Request(e);
                    continue;
                }
            };
            let status = resp.status();
            let raw = match resp.text() {
                Ok(t) => t,
                Err(e) => {
                    last_err = Error::Request(e);
                    continue;
                }
            };
            if !status.is_success() {
                last_err = Error::Http(format!("upload {status}: {}", &raw[..raw.len().min(300)]));
                continue;
            }
            let v: Value = match serde_json::from_str(&raw) {
                Ok(v) => v,
                Err(e) => {
                    last_err = Error::Parse(format!("upload non-json: {e}; {raw}"));
                    continue;
                }
            };
            if json_str(&v["zt"]) != "1" {
                last_err = Error::Parse(format!("upload failed: {raw}"));
                continue;
            }
            let mut file_id = String::new();
            let mut name = filename.to_string();
            if let Some(arr) = v.get("text").and_then(|t| t.as_array()) {
                if let Some(row) = arr.first() {
                    file_id = json_str(&row["id"]);
                    let n = json_str(&row["name"]);
                    let n2 = json_str(&row["name_all"]);
                    if !n.is_empty() {
                        name = n;
                    } else if !n2.is_empty() {
                        name = n2;
                    }
                }
            } else if let Some(obj) = v.get("text").and_then(|t| t.as_object()) {
                file_id = json_str(&obj["id"]);
                let n = json_str(&obj["name"]);
                let n2 = json_str(&obj["name_all"]);
                if !n.is_empty() {
                    name = n;
                } else if !n2.is_empty() {
                    name = n2;
                }
            }
            if file_id.is_empty() {
                file_id = json_str(&v["id"]);
                if file_id.is_empty() {
                    if let Some(info) = v.get("info").and_then(|i| i.as_object()) {
                        file_id = json_str(&info["id"]);
                    } else {
                        file_id = json_str(&v["info"]);
                    }
                }
            }
            return Ok(UploadResult {
                file_id,
                name,
                folder_id: folder_id.to_string(),
                raw_json: raw,
                parts: Vec::new(),
                orig_name: None,
                group_id: None,
            });
        }
        Err(last_err)
    }

    /// Best-effort load of file descriptions for list unescape.
    pub fn fetch_notes(&self, list: &[ListEntry]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        for e in list {
            if e.kind != EntryKind::File {
                continue;
            }
            if let Some(d) = &e.description {
                if d.contains("[lanzou-part]") {
                    out.insert(e.id.clone(), d.clone());
                    continue;
                }
            }
            if let Ok(desc) = self.get_file_describe(&e.id) {
                if !desc.is_empty() {
                    out.insert(e.id.clone(), desc);
                }
            }
        }
        out
    }
}

/// Removes a temporary upload file on drop.
struct TempGuard(Option<PathBuf>);
impl Drop for TempGuard {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = fs::remove_file(p);
        }
    }
}

struct MultiTempGuard(Vec<PathBuf>);
impl Drop for MultiTempGuard {
    fn drop(&mut self) {
        for p in self.0.drain(..) {
            let _ = fs::remove_file(p);
        }
    }
}

/// One chunk of a split upload.
#[derive(Debug, Clone)]
pub struct UploadPart {
    pub file_id: String,
    pub name: String,
    pub index: usize,
    pub total: usize,
    pub size: u64,
}

/// Result of a successful upload (single or multi-part).
#[derive(Debug, Clone)]
pub struct UploadResult {
    pub file_id: String,
    pub name: String,
    pub folder_id: String,
    pub raw_json: String,
    pub parts: Vec<UploadPart>,
    pub orig_name: Option<String>,
    pub group_id: Option<String>,
}

/// Display row after optional split-unescape grouping.
#[derive(Debug, Clone)]
pub struct DisplayEntry {
    pub kind: String, // DIR | FILE | SPLIT
    pub id: String,
    pub name: String,
    pub size: String,
    pub extra: String,
    pub parts: Vec<ListEntry>,
}

/// Group split parts (via description notes) into virtual rows.
pub fn unescape_list(
    list: &[ListEntry],
    notes: &HashMap<String, String>,
    enabled: bool,
) -> Vec<DisplayEntry> {
    if !enabled {
        let mut out: Vec<DisplayEntry> = list.iter().map(flat_entry).collect();
        apply_convert_notes(&mut out, notes);
        return out;
    }
    use std::collections::BTreeMap;
    let mut groups: BTreeMap<String, (crate::notes::PartMeta, Vec<ListEntry>)> = BTreeMap::new();
    let mut plain: Vec<ListEntry> = Vec::new();
    for e in list {
        if e.kind == EntryKind::Folder {
            plain.push(e.clone());
            continue;
        }
        let note = notes
            .get(&e.id)
            .cloned()
            .or_else(|| e.description.clone())
            .unwrap_or_default();
        if let Some(meta) = crate::notes::parse_part_note(&note) {
            let ent = groups
                .entry(meta.group_id.clone())
                .or_insert_with(|| (meta.clone(), Vec::new()));
            if ent.0.name.is_empty() && !meta.name.is_empty() {
                ent.0.name = meta.name.clone();
            }
            if meta.total > ent.0.total {
                ent.0.total = meta.total;
            }
            ent.1.push(e.clone());
        } else {
            plain.push(e.clone());
        }
    }
    let mut out: Vec<DisplayEntry> = plain.iter().map(flat_entry).collect();
    apply_convert_notes(&mut out, notes);
    for (gid, (meta, mut parts)) in groups {
        parts.sort_by_key(|p| {
            let n = notes
                .get(&p.id)
                .cloned()
                .or_else(|| p.description.clone())
                .unwrap_or_default();
            crate::notes::parse_part_note(&n)
                .map(|m| m.index)
                .unwrap_or(0)
        });
        let name = if meta.name.is_empty() {
            parts
                .first()
                .map(|p| p.name.clone())
                .unwrap_or_else(|| gid.clone())
        } else {
            meta.name.clone()
        };
        let mut total_size = 0u64;
        for p in &parts {
            let n = notes
                .get(&p.id)
                .cloned()
                .or_else(|| p.description.clone())
                .unwrap_or_default();
            if let Some(m) = crate::notes::parse_part_note(&n) {
                total_size += m.size;
            }
        }
        let first_id = parts
            .first()
            .map(|p| p.id.clone())
            .unwrap_or_default();
        out.push(DisplayEntry {
            kind: "SPLIT".into(),
            id: first_id,
            name,
            size: human_size(total_size),
            extra: format!("parts={}/{} group={gid}", parts.len(), meta.total),
            parts,
        });
    }
    out
}

fn flat_entry(e: &ListEntry) -> DisplayEntry {
    let (kind, extra) = if e.kind == EntryKind::Folder {
        (
            "DIR".into(),
            e.description.clone().unwrap_or_default(),
        )
    } else {
        ("FILE".into(), e.size.clone().unwrap_or_default())
    };
    DisplayEntry {
        kind,
        id: e.id.clone(),
        name: e.name.clone(),
        size: e.size.clone().unwrap_or_default(),
        extra,
        parts: Vec::new(),
    }
}

fn apply_convert_notes(rows: &mut [DisplayEntry], notes: &HashMap<String, String>) {
    for row in rows.iter_mut() {
        if row.kind != "FILE" {
            continue;
        }
        let note = notes.get(&row.id).cloned().unwrap_or_default();
        let Some(cm) = crate::notes::parse_convert_note(&note) else {
            continue;
        };
        if cm.name.is_empty() {
            continue;
        }
        let as_name = if cm.as_name.is_empty() {
            row.name.clone()
        } else {
            cm.as_name.clone()
        };
        row.name = cm.name;
        let hint = format!("as={} mode={}", as_name, cm.mode);
        if row.extra.is_empty() {
            row.extra = hint;
        } else {
            row.extra = format!("{}  {}", row.extra, hint);
        }
    }
}

fn human_size(n: u64) -> String {
    if n == 0 {
        return String::new();
    }
    const UNIT: f64 = 1024.0;
    let mut v = n as f64;
    let mut exp = 0;
    while v >= UNIT && exp < 5 {
        v /= UNIT;
        exp += 1;
    }
    if exp == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}B", ['K', 'M', 'G', 'T', 'P'][exp - 1])
    }
}

fn json_str(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

fn str_intercept(s: &str, start: &str, end: &str) -> String {
    if start.is_empty() {
        if end.is_empty() {
            return s.to_string();
        }
        return s.split(end).next().unwrap_or("").to_string();
    }
    let Some(i) = s.find(start) else {
        return String::new();
    };
    let i = i + start.len();
    if end.is_empty() {
        return s[i..].to_string();
    }
    let Some(j) = s[i..].find(end) else {
        return String::new();
    };
    s[i..i + j].to_string()
}


