//! Account / control-panel APIs (login, folder/file management).
//! Ported from `lanzou.class.php`. Credentials are never hard-coded.

use crate::error::{Error, Result};
use regex::Regex;
use reqwest::blocking::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, COOKIE, SET_COOKIE, USER_AGENT};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const DEFAULT_ACCOUNT_BASE: &str = "https://up.woozooo.com/";
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

    /// Login and persist cookie if cookie file is set.
    pub fn login(&mut self) -> Result<()> {
        let resp = self
            .http
            .post(format!("{}mlogin.php", self.base))
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
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
            return Err(Error::Parse(format!("login failed: {raw}")));
        }
        if self.cookie.is_empty() {
            return Err(Error::Parse("login ok but empty cookie".into()));
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
        match self.post_task("task=5&folder_id=-1") {
            Ok(raw) => {
                if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                    json_str(&v["zt"]) != "9"
                } else {
                    false
                }
            }
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
    pub fn list(&self, folder_id: &str) -> Result<Vec<ListEntry>> {
        let folder_id = if folder_id.is_empty() {
            "-1"
        } else {
            folder_id
        };
        let html = self.get_html(&format!("myfile.php?folder_id={folder_id}"))?;
        let mut out = Vec::new();

        // Prefer robust regex for folders
        let re_folder = Regex::new(
            r#"onclick="modify\((\d+)\)"[\s\S]*?<span>([^<]*)</span>[\s\S]*?<a href="([^"]*)"[\s\S]*?<div class="folders1">([^<]*)</div>"#,
        )
        .unwrap();
        for cap in re_folder.captures_iter(&html) {
            out.push(ListEntry {
                kind: EntryKind::Folder,
                id: cap[1].to_string(),
                name: cap[2].to_string(),
                url: Some(cap[3].to_string()),
                size: None,
                time: None,
                description: Some(cap[4].trim().to_string()),
            });
        }

        let raw = self.post_task(&format!("task=5&folder_id={folder_id}"))?;
        let v: Value = serde_json::from_str(&raw)
            .map_err(|e| Error::Parse(format!("list files json: {e}; {raw}")))?;
        if let Some(arr) = v.get("text").and_then(|t| t.as_array()) {
            for it in arr {
                let id = json_str(&it["id"]);
                let desc = self.get_file_describe(&id).ok();
                out.push(ListEntry {
                    kind: EntryKind::File,
                    id,
                    name: json_str(&it["name_all"]),
                    url: None,
                    size: Some(json_str(&it["size"])).filter(|s| !s.is_empty()),
                    time: Some(json_str(&it["time"])).filter(|s| !s.is_empty()),
                    description: desc,
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
