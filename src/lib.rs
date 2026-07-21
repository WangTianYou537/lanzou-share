//! Lanzou (蓝奏云) share link resolver and account manager.
//!
//! Includes public/password share resolve and control-panel APIs
//! (login / folder / file) ported from `lanzou.class.php`.

mod account;
mod acw;
mod config;
mod error;
mod notes;

pub use account::{
    is_upload_allowed_ext, unescape_list, Account, DisplayEntry, EntryKind, FileInfo, FolderInfo,
    ListEntry, UploadPart, UploadResult,
};
pub use acw::{calc_acw_sc_v2, extract_arg1, is_acw_challenge, solve_from_html, MASK, PERM};
pub use config::{
    config_keys, config_path_used, default_config_path, format_split_name, get_config,
    get_config_value, load_config, save_config, set_config_cache, set_config_value, Config,
};
pub use notes::{
    format_convert_note, format_part_note, format_raw_note, parse_convert_note, parse_file_note,
    parse_part_note, ConvertMeta, FileNote, PartMeta, NOTE_VERSION,
};
/// Crate / CLI version (from Cargo.toml).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub use error::{Error, Result};

use regex::Regex;
use reqwest::blocking::Client as HttpClient;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_LANGUAGE, CONTENT_TYPE, COOKIE, REFERER, SET_COOKIE, USER_AGENT};
use reqwest::redirect::Policy;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;
use url::Url;

const DEFAULT_UA: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const CDN_RISK_WAIT: Duration = Duration::from_millis(2200);

/// Options for [`Client::parse`].
#[derive(Debug, Clone)]
pub struct ParseOptions {
    pub password: Option<String>,
    pub resolve_direct: bool,
}

impl Default for ParseOptions {
    fn default() -> Self {
        Self {
            password: None,
            resolve_direct: true,
        }
    }
}

impl ParseOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn password(mut self, pwd: impl Into<String>) -> Self {
        self.password = Some(pwd.into());
        self
    }

    pub fn resolve_direct(mut self, v: bool) -> Self {
        self.resolve_direct = v;
        self
    }
}

/// Parsed share result.
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub fid: String,
    pub filename: Option<String>,
    pub password_protected: bool,
    pub cdn_domain: String,
    pub telecom: String,
    pub unicom: String,
    pub normal: String,
    pub direct: Option<String>,
    pub saved_path: Option<PathBuf>,
}

/// Lanzou HTTP client with simple cookie jar.
pub struct Client {
    http: HttpClient,
    cookies: HashMap<String, String>,
    origin: String,
    last_filename: Option<String>,
}

impl Client {
    pub fn new() -> Result<Self> {
        let mut headers = HeaderMap::new();
        headers.insert(USER_AGENT, HeaderValue::from_static(DEFAULT_UA));
        headers.insert(
            ACCEPT,
            HeaderValue::from_static(
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            ),
        );
        headers.insert(
            ACCEPT_LANGUAGE,
            HeaderValue::from_static("zh-CN,zh;q=0.9,en;q=0.8"),
        );

        let http = HttpClient::builder()
            .default_headers(headers)
            .redirect(Policy::limited(10))
            .timeout(Duration::from_secs(30))
            .build()?;

        Ok(Self {
            http,
            cookies: HashMap::new(),
            origin: String::new(),
            last_filename: None,
        })
    }

    fn origin_of(url: &str) -> Result<String> {
        let u = Url::parse(url)?;
        Ok(format!(
            "{}://{}",
            u.scheme(),
            u.host_str().unwrap_or_default()
        ))
    }

    fn store_cookies(&mut self, headers: &HeaderMap) {
        for val in headers.get_all(SET_COOKIE) {
            if let Ok(s) = val.to_str() {
                if let Some(pair) = s.split(';').next() {
                    if let Some((k, v)) = pair.split_once('=') {
                        self.cookies
                            .insert(k.trim().to_string(), v.trim().to_string());
                    }
                }
            }
        }
    }

    fn cookie_header(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        Some(
            self.cookies
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
    }

    fn get_html(&mut self, url: &str, referer: Option<&str>) -> Result<String> {
        let mut req = self.http.get(url);
        if let Some(r) = referer {
            req = req.header(REFERER, r);
        }
        if let Some(c) = self.cookie_header() {
            req = req.header(COOKIE, c);
        }
        let resp = req.send()?;
        self.store_cookies(resp.headers());
        if !resp.status().is_success() {
            return Err(Error::Http(format!("GET {} -> {}", url, resp.status())));
        }
        let mut text = resp.text()?;

        if is_acw_challenge(&text) {
            let (_arg1, token) = solve_from_html(&text)?;
            self.cookies
                .insert("acw_sc__v2".into(), token);
            let mut req2 = self.http.get(url);
            if let Some(r) = referer {
                req2 = req2.header(REFERER, r);
            }
            if let Some(c) = self.cookie_header() {
                req2 = req2.header(COOKIE, c);
            }
            let resp2 = req2.send()?;
            self.store_cookies(resp2.headers());
            if !resp2.status().is_success() {
                return Err(Error::Http(format!(
                    "GET {} after acw -> {}",
                    url,
                    resp2.status()
                )));
            }
            text = resp2.text()?;
            if is_acw_challenge(&text) {
                return Err(Error::Acw("still on challenge page after cookie".into()));
            }
        }
        Ok(text)
    }

    pub fn is_password_protected(html: &str) -> bool {
        const MARKERS: &[&str] = &[
            "id=\"passwddiv\"",
            "id='passwddiv'",
            "id=\"pwd\"",
            "function down_p(",
            "passwddiv-input",
            "输入密码",
        ];
        MARKERS.iter().any(|m| html.contains(m))
    }

    fn extract_pwd_sign(html: &str) -> Option<String> {
        let re = Regex::new(r"'sign'\s*:\s*'([^']+)'").ok()?;
        let mut valid = Vec::new();
        for cap in re.captures_iter(html) {
            let s = cap.get(1)?.as_str();
            if s.len() > 20 && !s.contains('<') && !s.contains('>') {
                valid.push(s.to_string());
            }
        }
        valid.pop()
    }

    fn extract_pwd_fid(html: &str) -> Option<String> {
        let re = Regex::new(r"ajaxm\.php\?file=(\d+)").ok()?;
        let mut last = None;
        for cap in re.captures_iter(html) {
            last = cap.get(1).map(|m| m.as_str().to_string());
        }
        if last.is_some() {
            return last;
        }
        let re2 = Regex::new(r"[?&]f=(\d+)").ok()?;
        re2.captures(html)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    fn re_first(html: &str, pat: &str) -> Option<String> {
        Regex::new(pat)
            .ok()?
            .captures(html)
            .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
    }

    /// Full parse pipeline.
    pub fn parse(&mut self, share_url: &str, opts: ParseOptions) -> Result<ParseResult> {
        self.last_filename = None;
        self.origin = Self::origin_of(share_url)?;

        let html = self.get_html(share_url, None)?;

        let (fid, links, password_protected) = if Self::is_password_protected(&html) {
            let password = opts
                .password
                .as_deref()
                .filter(|s| !s.is_empty())
                .ok_or(Error::PasswordRequired)?;

            let fid = Self::extract_pwd_fid(&html)
                .ok_or_else(|| Error::Parse("password page: missing fid".into()))?;
            let sign = Self::extract_pwd_sign(&html)
                .ok_or_else(|| Error::Parse("password page: missing sign".into()))?;
            let mut kdns =
                Self::re_first(&html, r"var\s+kdns\s*=\s*(\d+)").unwrap_or_else(|| "1".into());
            if let Some(k) = Self::re_first(&html, r"var\s+killdns\s*=\s*(\d+)") {
                kdns = k;
            }
            let links = self.ajaxm_password(&fid, &sign, &kdns, password, share_url)?;
            (fid, links, true)
        } else {
            let fid = Self::re_first(&html, r"var\s+fid\s*=\s*(\d+)\s*;")
                .or_else(|| Self::re_first(&html, r#"data-id="(\d+)""#))
                .or_else(|| Self::re_first(&html, r"fid\s*=\s*'(\d+)'"))
                .ok_or_else(|| Error::Parse("public page: missing fid".into()))?;

            let fn_path = Self::re_first(&html, r#"iframe[^>]*src="(/fn\?[^"]*)""#)
                .or_else(|| Self::re_first(&html, r"src='(/fn\?[^']*)'"))
                .ok_or_else(|| Error::Parse("public page: missing fn path".into()))?;

            if let Some(name) =
                Self::re_first(&html, r"<title>(.*?)(?:\s*-\s*蓝奏云)?</title>")
            {
                let name = name.trim().to_string();
                if name != "文件" && name != "蓝奏云" {
                    self.last_filename = Some(name);
                }
            }

            let fn_url = format!("{}{}", self.origin, fn_path);
            let fn_html = self.get_html(&fn_url, Some(&format!("{}/", self.origin)))?;
            let wp_sign = Self::re_first(&fn_html, r"var\s+wp_sign\s*=\s*'([^']*)'")
                .ok_or_else(|| Error::Parse("fn page: missing wp_sign".into()))?;
            let ajaxdata = Self::re_first(&fn_html, r"var\s+ajaxdata\s*=\s*'([^']*)'")
                .ok_or_else(|| Error::Parse("fn page: missing ajaxdata".into()))?;
            let mut kdns =
                Self::re_first(&fn_html, r"var\s+kdns\s*=\s*(\d+)").unwrap_or_else(|| "1".into());
            if let Some(k) = Self::re_first(&fn_html, r"var\s+killdns\s*=\s*(\d+)") {
                kdns = k;
            }

            let links = self.ajaxm_public(&fid, &wp_sign, &ajaxdata, &kdns, &fn_path)?;
            (fid, links, false)
        };

        let mut result = ParseResult {
            fid,
            filename: self.last_filename.clone(),
            password_protected,
            cdn_domain: links.dom.clone(),
            telecom: links.telecom.clone(),
            unicom: links.unicom.clone(),
            normal: links.normal.clone(),
            direct: None,
            saved_path: None,
        };

        if opts.resolve_direct {
            result.direct = Some(
                self.resolve_direct_url(&links.telecom)
                    .or_else(|_| self.resolve_direct_url(&links.normal))?,
            );
        }

        Ok(result)
    }

    fn ajaxm_public(
        &mut self,
        fid: &str,
        wp_sign: &str,
        ajaxdata: &str,
        kdns: &str,
        fn_path: &str,
    ) -> Result<LinkSet> {
        let api = format!("{}/ajaxm.php?file={fid}", self.origin);
        let body = [
            ("action", "downprocess"),
            ("websignkey", ajaxdata),
            ("signs", ajaxdata),
            ("sign", wp_sign),
            ("websign", ""),
            ("kd", kdns),
            ("ves", "1"),
        ];
        let referer = format!("{}{}", self.origin, fn_path);
        self.post_ajaxm(&api, &body, &referer, kdns)
    }

    fn ajaxm_password(
        &mut self,
        fid: &str,
        sign: &str,
        kdns: &str,
        password: &str,
        share_url: &str,
    ) -> Result<LinkSet> {
        let api = format!("{}/ajaxm.php?file={fid}", self.origin);
        let body = [
            ("action", "downprocess"),
            ("sign", sign),
            ("kd", kdns),
            ("p", password),
        ];
        self.post_ajaxm(&api, &body, share_url, kdns)
    }

    fn post_ajaxm(
        &mut self,
        api: &str,
        body: &[(&str, &str)],
        referer: &str,
        kdns: &str,
    ) -> Result<LinkSet> {
        let mut req = self
            .http
            .post(api)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(REFERER, referer)
            .header("Origin", &self.origin)
            .header("X-Requested-With", "XMLHttpRequest")
            .form(body);
        if let Some(c) = self.cookie_header() {
            req = req.header(COOKIE, c);
        }
        let resp = req.send()?;
        self.store_cookies(resp.headers());
        let status = resp.status();
        let raw = resp
            .text()
            .map_err(|e| Error::Parse(format!("ajaxm read: {e}")))?;
        if !status.is_success() {
            return Err(Error::Http(format!(
                "ajaxm {api} -> {status} body={}",
                &raw[..raw.len().min(200)]
            )));
        }
        let data: AjaxmResp = serde_json::from_str(&raw).map_err(|e| {
            Error::Parse(format!(
                "ajaxm non-json: {e}; body={}",
                &raw[..raw.len().min(300)]
            ))
        })?;
        if data.zt_string() != "1" {
            return Err(Error::Parse(format!(
                "ajaxm failed zt={} info={:?}",
                data.zt_string(),
                data.inf_string()
            )));
        }
        let mut dom = data
            .dom_string()
            .ok_or_else(|| Error::Parse("ajaxm missing dom".into()))?;
        let url = data
            .url_string()
            .ok_or_else(|| Error::Parse("ajaxm missing url".into()))?;
        if let Some(inf) = data.inf_string() {
            let inf = inf.trim().to_string();
            // public shares sometimes return numeric noise in inf
            let looks_name = inf.contains('.')
                || inf.contains('_')
                || inf.contains('-')
                || inf.contains(' ')
                || !inf.chars().all(|c| c.is_ascii_digit());
            if !inf.is_empty() && inf != "文件" && looks_name {
                self.last_filename = Some(inf);
            }
        }
        if kdns == "0" {
            dom = "https://slssctm.dmpdmp.com".into();
        }
        let base = format!("{dom}/file/{url}");
        let normal = if base.contains('?') {
            format!("{base}&toolsdown")
        } else {
            format!("{base}?toolsdown")
        };
        Ok(LinkSet {
            dom,
            telecom: base.clone(),
            unicom: base,
            normal,
        })
    }

    /// Resolve CDN pseudo URL to downloadable URL.
    pub fn resolve_direct_url(&mut self, cdn_url: &str) -> Result<String> {
        let referer = if self.origin.is_empty() {
            "https://www.lanzou.com/".to_string()
        } else {
            format!("{}/", self.origin)
        };
        let mut req = self
            .http
            .get(cdn_url)
            .header(REFERER, referer)
            .header(ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
            .header(ACCEPT, "*/*");
        if let Some(c) = self.cookie_header() {
            req = req.header(COOKIE, c);
        }
        let resp = req.send()?;
        self.store_cookies(resp.headers());
        if !resp.status().is_success() {
            return Err(Error::Cdn(format!("status {}", resp.status())));
        }
        let ct = resp
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let mut bytes = Vec::new();
        let mut r = resp;
        let mut take = [0u8; 256];
        let n = r.read(&mut take)?;
        bytes.extend_from_slice(&take[..n]);
        if looks_like_file(&bytes, &ct) {
            return Ok(cdn_url.to_string());
        }
        r.read_to_end(&mut bytes)?;
        let text = String::from_utf8_lossy(&bytes);
        if is_cdn_risk_page(&text) {
            return self.resolve_via_ajax(cdn_url, &text);
        }
        Err(Error::Cdn(format!(
            "unknown CDN response ct={ct} head={:?}",
            &bytes[..bytes.len().min(60)]
        )))
    }

    fn resolve_via_ajax(&mut self, cdn_url: &str, risk_html: &str) -> Result<String> {
        let file_token = Self::re_first(risk_html, r"'file'\s*:\s*'([^']+)'")
            .ok_or_else(|| Error::Cdn("risk page missing file".into()))?;
        let sign = Self::re_first(risk_html, r"'sign'\s*:\s*'([^']+)'")
            .ok_or_else(|| Error::Cdn("risk page missing sign".into()))?;
        let ajax_url = Url::parse(cdn_url)?.join("ajax.php")?;
        let origin = Self::origin_of(cdn_url)?;

        let mut last_err = String::new();
        for _ in 0..2 {
            thread::sleep(CDN_RISK_WAIT);
            let mut req = self
                .http
                .post(ajax_url.as_str())
                .header(REFERER, cdn_url)
                .header("Origin", &origin)
                .header("X-Requested-With", "XMLHttpRequest")
                .header(
                    CONTENT_TYPE,
                    "application/x-www-form-urlencoded; charset=UTF-8",
                )
                .header(ACCEPT, "application/json, text/javascript, */*; q=0.01")
                .form(&[
                    ("file", file_token.as_str()),
                    ("el", "2"),
                    ("sign", sign.as_str()),
                ]);
            if let Some(c) = self.cookie_header() {
                req = req.header(COOKIE, c);
            }
            let resp = req.send()?;
            self.store_cookies(resp.headers());
            let data: AjaxRiskResp = match resp.json() {
                Ok(d) => d,
                Err(e) => {
                    last_err = format!("non-json: {e}");
                    continue;
                }
            };
            let zt = data.zt_string();
            let final_url = data.url.unwrap_or_default();
            if zt == "1" && !final_url.is_empty() && !final_url.starts_with('?') {
                let final_url = if let Some(rest) = final_url.strip_prefix("//") {
                    format!("https://{rest}")
                } else if final_url.starts_with('/') {
                    format!("{origin}{final_url}")
                } else {
                    final_url
                };
                return Ok(final_url);
            }
            last_err = format!("zt={zt} url={final_url}");
        }
        Err(Error::Cdn(format!("ajax risk failed: {last_err}")))
    }

    /// Download URL to local path; returns saved path.
    pub fn download(
        &mut self,
        url: &str,
        dest_dir: impl AsRef<Path>,
        filename: Option<&str>,
        referer: Option<&str>,
    ) -> Result<PathBuf> {
        let dest_dir = dest_dir.as_ref();
        std::fs::create_dir_all(dest_dir)?;

        let referer = referer.map(|s| s.to_string()).unwrap_or_else(|| {
            if self.origin.is_empty() {
                "https://www.lanzou.com/".into()
            } else {
                format!("{}/", self.origin)
            }
        });

        let mut req = self
            .http
            .get(url)
            .header(REFERER, referer)
            .header(ACCEPT, "*/*")
            .header(ACCEPT_LANGUAGE, "zh-CN,zh;q=0.9,en;q=0.8")
            .timeout(Duration::from_secs(120));
        if let Some(c) = self.cookie_header() {
            req = req.header(COOKIE, c);
        }
        let resp = req.send()?;
        self.store_cookies(resp.headers());
        if !resp.status().is_success() {
            return Err(Error::Http(format!("download status {}", resp.status())));
        }

        let name = filename
            .map(|s| s.to_string())
            .or_else(|| filename_from_cd(resp.headers().get("content-disposition")))
            .or_else(|| filename_from_url(url))
            .or_else(|| self.last_filename.clone())
            .unwrap_or_else(|| format!("lanzou_{}.bin", now_ts()));

        let name = Path::new(&name)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("download.bin")
            .to_string();

        let mut out_path = dest_dir.join(&name);
        if out_path.exists() {
            let stem = out_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("file");
            let ext = out_path
                .extension()
                .and_then(|s| s.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_default();
            let mut i = 1;
            loop {
                let candidate = dest_dir.join(format!("{stem}({i}){ext}"));
                if !candidate.exists() {
                    out_path = candidate;
                    break;
                }
                i += 1;
            }
        }

        let total = resp.content_length().unwrap_or(0);
        let mut file = File::create(&out_path)?;
        let mut reader = resp;
        let mut buf = [0u8; 64 * 1024];
        let mut read: u64 = 0;
        let mut last = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let label = name.clone();
        if total == 0 {
            eprint!("\r[download] {label}  ...");
        }
        loop {
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])?;
            read += n as u64;
            if total > 0 {
                let now = std::time::Instant::now();
                if now.duration_since(last).as_millis() >= 200 || read >= total {
                    last = now;
                    let pct = read as f64 * 100.0 / total as f64;
                    eprint!(
                        "\r[download] {label}  {pct:5.1}%  {}/{}  ",
                        human_bytes(read),
                        human_bytes(total)
                    );
                }
            }
        }
        if total > 0 {
            eprintln!(
                "\r[download] {label}  100.0%  {}/{}          ",
                human_bytes(total),
                human_bytes(total)
            );
        } else {
            eprintln!("\r[download] {label}  done                    ");
        }
        Ok(out_path)
    }
}

#[derive(Debug)]
struct LinkSet {
    dom: String,
    telecom: String,
    unicom: String,
    normal: String,
}

#[derive(Debug, Deserialize)]
struct AjaxmResp {
    #[serde(default)]
    zt: serde_json::Value,
    #[serde(default)]
    dom: serde_json::Value,
    #[serde(default)]
    url: serde_json::Value,
    #[serde(default)]
    inf: serde_json::Value,
}

impl AjaxmResp {
    fn zt_string(&self) -> String {
        json_to_string(&self.zt)
    }

    fn dom_string(&self) -> Option<String> {
        let s = json_to_string(&self.dom);
        if s.is_empty() || s == "null" {
            None
        } else {
            Some(s)
        }
    }

    fn url_string(&self) -> Option<String> {
        let s = json_to_string(&self.url);
        if s.is_empty() || s == "null" {
            None
        } else {
            Some(s)
        }
    }

    fn inf_string(&self) -> Option<String> {
        let s = json_to_string(&self.inf);
        if s.is_empty() || s == "null" {
            None
        } else {
            Some(s)
        }
    }
}

fn json_to_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

#[derive(Debug, Deserialize)]
struct AjaxRiskResp {
    #[serde(default)]
    zt: serde_json::Value,
    #[serde(default)]
    url: Option<String>,
}

impl AjaxRiskResp {
    fn zt_string(&self) -> String {
        match &self.zt {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            other => other.to_string(),
        }
    }
}

fn is_cdn_risk_page(html: &str) -> bool {
    (html.contains("ajax.php") && html.contains("down_r"))
        || (html.contains("系统发现您的网络异常") && html.contains("ajax.php"))
}

fn looks_like_file(chunk: &[u8], content_type: &str) -> bool {
    let ct = content_type.to_ascii_lowercase();
    if ct.contains("octet-stream")
        || ct.contains("zip")
        || ct.contains("msword")
        || ct.contains("excel")
    {
        return true;
    }
    if ct.contains("application/") && !ct.contains("html") && !ct.contains("json") {
        return true;
    }
    if chunk.len() >= 2 && &chunk[..2] == b"PK" {
        return true;
    }
    if chunk.starts_with(&[0x1f, 0x8b]) {
        return true;
    }
    if chunk.len() >= 4 && chunk[..4] == [0xd0, 0xcf, 0x11, 0xe0] {
        return true;
    }
    if chunk.starts_with(b"%PDF") {
        return true;
    }
    let head = String::from_utf8_lossy(&chunk[..chunk.len().min(20)]).to_ascii_lowercase();
    let t = head.trim_start();
    !(t.starts_with("<!doctype") || t.starts_with("<html") || t.starts_with("<script"))
}

fn filename_from_cd(h: Option<&reqwest::header::HeaderValue>) -> Option<String> {
    let s = h?.to_str().ok()?;
    if let Some(rest) = s.split("filename*=UTF-8''").nth(1) {
        let v = rest.split(';').next()?.trim().trim_matches('"');
        return Some(urlencoding::decode(v).ok()?.into_owned());
    }
    if let Some(idx) = s.find("filename=") {
        let v = s[idx + 9..].split(';').next()?.trim().trim_matches('"');
        return Some(v.to_string());
    }
    None
}

fn filename_from_url(url: &str) -> Option<String> {
    let u = Url::parse(url).ok()?;
    for key in ["fileName", "filename", "fn", "name"] {
        if let Some(v) = u
            .query_pairs()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.into_owned())
        {
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    let name = Path::new(u.path()).file_name()?.to_str()?.to_string();
    if name.is_empty() || name == "file" || name == "ajax.php" {
        None
    } else {
        Some(name)
    }
}

fn now_ts() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}


fn human_bytes(n: u64) -> String {
    if n < 1024 {
        return format!("{n}B");
    }
    let mut f = n as f64;
    for u in ["KB", "MB", "GB", "TB"] {
        f /= 1024.0;
        if f < 1024.0 {
            return format!("{f:.1}{u}");
        }
    }
    format!("{:.1}PB", f / 1024.0)
}
