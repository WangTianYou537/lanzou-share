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
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_ACCOUNT_BASE: &str = "https://pc.woozooo.com/";
const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

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

    /// Upload a local file via `fileup.php` (task=1 multipart).
    /// Protocol aligned with common Lanzou web clients.
    pub fn upload(&self, local_path: impl AsRef<Path>, folder_id: &str) -> Result<UploadResult> {
        let local_path = local_path.as_ref();
        let folder_id = if folder_id.is_empty() { "-1" } else { folder_id };
        if !local_path.is_file() {
            return Err(Error::Parse(format!("not a file: {}", local_path.display())));
        }
        let filename = local_path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| Error::Parse("invalid filename".into()))?
            .to_string();

        let mut urls = vec![format!("{}fileup.php", self.base)];
        if self.base.contains("up.woozooo.com") {
            urls.push("https://pc.woozooo.com/fileup.php".into());
        } else if self.base.contains("pc.woozooo.com") {
            urls.push("https://up.woozooo.com/fileup.php".into());
        }

        let mut last_err = Error::Http("upload failed".into());
        for up_url in urls {
            let mut file = File::open(local_path)?;
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            let part = Part::bytes(buf)
                .file_name(filename.clone())
                .mime_str("application/octet-stream")
                .map_err(|e| Error::Http(e.to_string()))?;
            let form = Form::new()
                .text("task", "1")
                .text("vie", "2")
                .text("ve", "2")
                .text("id", "WU_FILE_0")
                .text("folder_id_bb_n", folder_id.to_string())
                .text("name", filename.clone())
                .part("upload_file", part);

            let mut req = self
                .http
                .post(&up_url)
                .timeout(Duration::from_secs(3600))
                .header(USER_AGENT, DEFAULT_UA)
                .header(reqwest::header::ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
                .header(reqwest::header::REFERER, &self.base)
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
            let mut name = filename.clone();
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
            }
            return Ok(UploadResult {
                file_id,
                name,
                folder_id: folder_id.to_string(),
                raw_json: raw,
            });
        }
        Err(last_err)
    }
}

/// Result of a successful upload.
#[derive(Debug, Clone)]
pub struct UploadResult {
    pub file_id: String,
    pub name: String,
    pub folder_id: String,
    pub raw_json: String,
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


