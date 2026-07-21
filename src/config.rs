//! User config for upload conversion / splitting / list display.
//! Stored as JSON at `~/.lanzou/config.json` by default.

use crate::error::{Error, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Process-wide preferences.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Auto-convert blocked suffixes before upload.
    #[serde(default = "default_true")]
    pub suffix_auto_convert: bool,
    /// Target extension without leading dot (default `zip`).
    #[serde(default = "default_suffix_name")]
    pub suffix_name: String,
    /// `zip` = real compress; `rename` = only change/append extension.
    #[serde(default = "default_suffix_mode")]
    pub suffix_mode: String,
    /// Split files larger than `split_size_mb`.
    #[serde(default = "default_true")]
    pub split_enable: bool,
    /// Chunk size in MB (1..=100, default 90).
    #[serde(default = "default_split_size")]
    pub split_size_mb: u32,
    /// Part name template. Default `{name}_s{index:03d}.{suffix}`.
    /// Avoid `*partNNN*` names — Lanzou CDN returns offline ERROR:102 on large files;
    /// also avoid `{name}.part{index}.zip` (server error 7071).
    #[serde(default = "default_split_format")]
    pub split_name_format: String,
    /// Write part metadata into file description after upload.
    #[serde(default = "default_true")]
    pub split_note: bool,
    /// Group split parts when listing.
    #[serde(default = "default_true")]
    pub list_unescape: bool,
}

fn default_true() -> bool {
    true
}
fn default_suffix_name() -> String {
    "zip".into()
}
fn default_suffix_mode() -> String {
    "zip".into()
}
fn default_split_size() -> u32 {
    90
}
fn default_split_format() -> String {
    "{name}_s{index:03d}.{suffix}".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            suffix_auto_convert: true,
            suffix_name: default_suffix_name(),
            suffix_mode: default_suffix_mode(),
            split_enable: true,
            split_size_mb: 90,
            split_name_format: default_split_format(),
            split_note: true,
            list_unescape: true,
        }
    }
}

impl Config {
    pub fn normalize(&mut self) {
        self.suffix_name = self
            .suffix_name
            .trim()
            .trim_start_matches('.')
            .to_ascii_lowercase();
        if self.suffix_name.is_empty() {
            self.suffix_name = "zip".into();
        }
        let mode = self.suffix_mode.to_ascii_lowercase();
        self.suffix_mode = if mode == "rename" {
            "rename".into()
        } else {
            "zip".into()
        };
        if self.split_size_mb < 1 {
            self.split_size_mb = 90;
        }
        if self.split_size_mb > 100 {
            self.split_size_mb = 100;
        }
        if self.split_name_format.trim().is_empty() {
            self.split_name_format = default_split_format();
        }
    }
}

static CONFIG: OnceLock<Mutex<(Config, PathBuf)>> = OnceLock::new();

/// Default path: `$LANZOU_CONFIG` or `~/.lanzou/config.json`.
pub fn default_config_path() -> PathBuf {
    if let Ok(v) = std::env::var("LANZOU_CONFIG") {
        if !v.is_empty() {
            return PathBuf::from(v);
        }
    }
    if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
        return PathBuf::from(home).join(".lanzou").join("config.json");
    }
    PathBuf::from("./lanzou.config.json")
}

/// Load config from path (missing file → defaults).
pub fn load_config(path: impl AsRef<Path>) -> Result<Config> {
    let path = path.as_ref();
    let mut cfg = Config::default();
    match fs::read_to_string(path) {
        Ok(s) => {
            cfg = serde_json::from_str(&s).map_err(|e| Error::Parse(format!("config json: {e}")))?;
            cfg.normalize();
            Ok(cfg)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(cfg),
        Err(e) => Err(Error::Io(e)),
    }
}

/// Save config to path, creating parent dirs.
pub fn save_config(path: impl AsRef<Path>, mut cfg: Config) -> Result<()> {
    cfg.normalize();
    let path = path.as_ref();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let s = serde_json::to_string_pretty(&cfg).map_err(|e| Error::Parse(e.to_string()))?;
    fs::write(path, s + "\n")?;
    // update cache
    let slot = CONFIG.get_or_init(|| Mutex::new((Config::default(), default_config_path())));
    let mut g = slot.lock().unwrap();
    *g = (cfg, path.to_path_buf());
    Ok(())
}

/// Process-wide cached config (loads once from default path).
pub fn get_config() -> Config {
    let slot = CONFIG.get_or_init(|| {
        let path = default_config_path();
        let cfg = load_config(&path).unwrap_or_default();
        Mutex::new((cfg, path))
    });
    slot.lock().unwrap().0.clone()
}

/// Path used by the cache / last save.
pub fn config_path_used() -> PathBuf {
    let slot = CONFIG.get_or_init(|| {
        let path = default_config_path();
        let cfg = load_config(&path).unwrap_or_default();
        Mutex::new((cfg, path))
    });
    slot.lock().unwrap().1.clone()
}

/// Replace in-memory config (no disk write).
pub fn set_config_cache(mut cfg: Config) {
    cfg.normalize();
    let slot = CONFIG.get_or_init(|| Mutex::new((Config::default(), default_config_path())));
    let mut g = slot.lock().unwrap();
    g.0 = cfg;
}

/// Settable keys with short descriptions.
pub fn config_keys() -> &'static [(&'static str, &'static str)] {
    &[
        (
            "suffix_auto_convert",
            "bool  auto convert unsupported suffix (default true)",
        ),
        (
            "suffix_name",
            "string target extension, no dot (default zip)",
        ),
        (
            "suffix_mode",
            "zip|rename  zip=compress, rename=only change suffix",
        ),
        ("split_enable", "bool  split large files (default true)"),
        (
            "split_size_mb",
            "int   chunk size in MB, 1..100 (default 90)",
        ),
        ("split_name_format", "string part name template"),
        (
            "split_note",
            "bool  write part metadata to file description",
        ),
        (
            "list_unescape",
            "bool  group split parts in ls (default true)",
        ),
    ]
}

pub fn set_config_value(mut cfg: Config, key: &str, value: &str) -> Result<Config> {
    let key = key.trim().to_ascii_lowercase();
    let value = value.trim();
    match key.as_str() {
        "suffix_auto_convert" => cfg.suffix_auto_convert = parse_bool(value)?,
        "suffix_name" => {
            let v = value.trim_start_matches('.').to_ascii_lowercase();
            if v.is_empty() {
                return Err(Error::Parse("suffix_name cannot be empty".into()));
            }
            cfg.suffix_name = v;
        }
        "suffix_mode" => {
            let v = value.to_ascii_lowercase();
            if v != "zip" && v != "rename" {
                return Err(Error::Parse("suffix_mode must be zip or rename".into()));
            }
            cfg.suffix_mode = v;
        }
        "split_enable" => cfg.split_enable = parse_bool(value)?,
        "split_size_mb" => {
            let n: u32 = value
                .parse()
                .map_err(|_| Error::Parse("split_size_mb must be integer 1..100".into()))?;
            if !(1..=100).contains(&n) {
                return Err(Error::Parse("split_size_mb must be integer 1..100".into()));
            }
            cfg.split_size_mb = n;
        }
        "split_name_format" => {
            if value.is_empty() {
                return Err(Error::Parse("split_name_format cannot be empty".into()));
            }
            cfg.split_name_format = value.into();
        }
        "split_note" => cfg.split_note = parse_bool(value)?,
        "list_unescape" => cfg.list_unescape = parse_bool(value)?,
        _ => return Err(Error::Parse(format!("unknown config key: {key}"))),
    }
    cfg.normalize();
    Ok(cfg)
}

pub fn get_config_value(cfg: &Config, key: &str) -> Result<String> {
    let key = key.trim().to_ascii_lowercase();
    Ok(match key.as_str() {
        "suffix_auto_convert" => cfg.suffix_auto_convert.to_string(),
        "suffix_name" => cfg.suffix_name.clone(),
        "suffix_mode" => cfg.suffix_mode.clone(),
        "split_enable" => cfg.split_enable.to_string(),
        "split_size_mb" => cfg.split_size_mb.to_string(),
        "split_name_format" => cfg.split_name_format.clone(),
        "split_note" => cfg.split_note.to_string(),
        "list_unescape" => cfg.list_unescape.to_string(),
        _ => return Err(Error::Parse(format!("unknown config key: {key}"))),
    })
}

fn parse_bool(s: &str) -> Result<bool> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "t" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "f" | "no" | "n" | "off" => Ok(false),
        _ => Err(Error::Parse(format!(
            "invalid bool: {s} (use true/false)"
        ))),
    }
}

/// Build a part filename from template.
pub fn format_split_name(format: &str, orig_base: &str, index: usize, total: usize, suffix: &str) -> String {
    let ext = Path::new(orig_base)
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let name = Path::new(orig_base)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(orig_base);
    let mut out = format.to_string();
    out = replace_index_token(&out, index);
    out = out.replace("{total}", &total.to_string());
    out = out.replace("{name}", name);
    out = out.replace("{ext}", &ext);
    out = out.replace("{suffix}", suffix);
    out
}

fn replace_index_token(s: &str, index: usize) -> String {
    let mut out = String::new();
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if s[i..].starts_with("{index") {
            let rest = &s[i + 6..];
            if rest.starts_with('}') {
                out.push_str(&index.to_string());
                i += 7;
                continue;
            }
            if let Some(stripped) = rest.strip_prefix(':') {
                if let Some(end) = stripped.find('}') {
                    let spec = &stripped[..end];
                    if let Some(w) = spec.strip_suffix('d') {
                        let width: usize = w.parse().unwrap_or(0);
                        if width > 0 {
                            out.push_str(&format!("{index:0width$}"));
                        } else {
                            out.push_str(&index.to_string());
                        }
                    } else {
                        out.push_str(&index.to_string());
                    }
                    i += 6 + 1 + end + 1; // {index + : + spec + }
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}
