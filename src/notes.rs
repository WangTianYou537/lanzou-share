//! Unified JSON file notes stored in Lanzou descriptions.
//!
//! ```json
//! {"v":1,"kind":"convert","name":"a.dex","as":"a.dex.zip","mode":"zip","suffix":"zip","size":20}
//! {"v":1,"kind":"part","id":"...","name":"big.bin","index":1,"total":3,"size":1048576}
//! ```
//!
//! Legacy plain-text markers (`[lanzou-convert]` / `[lanzou-part]`) are still parsed.

use serde::{Deserialize, Serialize};

/// Unified note schema written to file descriptions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileNote {
    #[serde(default = "default_v")]
    pub v: u32,
    pub kind: String, // convert | part
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "as")]
    pub as_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub suffix: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub index: usize,
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub total: usize,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub size: u64,
}

fn default_v() -> u32 {
    1
}
fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}
fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

/// Description written to each uploaded part (JSON).
pub fn format_part_note(group_id: &str, orig_name: &str, index: usize, total: usize, size: u64) -> String {
    serde_json::to_string(&FileNote {
        v: 1,
        kind: "part".into(),
        id: group_id.into(),
        name: orig_name.into(),
        index,
        total,
        size,
        ..Default::default()
    })
    .unwrap_or_default()
}

/// Description written when a suffix was converted (JSON).
pub fn format_convert_note(
    orig_name: &str,
    upload_name: &str,
    mode: &str,
    suffix: &str,
    size: u64,
) -> String {
    serde_json::to_string(&FileNote {
        v: 1,
        kind: "convert".into(),
        name: orig_name.into(),
        as_name: upload_name.into(),
        mode: mode.into(),
        suffix: suffix.into(),
        size,
        ..Default::default()
    })
    .unwrap_or_default()
}

/// Parsed part metadata from a file description.
#[derive(Debug, Clone, Default)]
pub struct PartMeta {
    pub group_id: String,
    pub name: String,
    pub index: usize,
    pub total: usize,
    pub size: u64,
}

/// Parsed convert metadata from a file description.
#[derive(Debug, Clone, Default)]
pub struct ConvertMeta {
    pub name: String,
    pub as_name: String,
    pub mode: String,
    pub suffix: String,
    pub size: u64,
}

/// Parse unified note (JSON first, then legacy text).
pub fn parse_file_note(desc: &str) -> Option<FileNote> {
    let desc = html_unescape(desc.trim());
    if desc.is_empty() {
        return None;
    }
    if let Some(i) = desc.find('{') {
        if let Some(j) = desc.rfind('}') {
            if j > i {
                if let Ok(mut n) = serde_json::from_str::<FileNote>(&desc[i..=j]) {
                    if !n.kind.is_empty() {
                        if n.v == 0 {
                            n.v = 1;
                        }
                        return Some(n);
                    }
                }
            }
        }
    }
    if let Some(cm) = parse_legacy_convert(&desc) {
        return Some(FileNote {
            v: 1,
            kind: "convert".into(),
            name: cm.name,
            as_name: cm.as_name,
            mode: cm.mode,
            suffix: cm.suffix,
            size: cm.size,
            ..Default::default()
        });
    }
    if let Some(pm) = parse_legacy_part(&desc) {
        return Some(FileNote {
            v: 1,
            kind: "part".into(),
            id: pm.group_id,
            name: pm.name,
            index: pm.index,
            total: pm.total,
            size: pm.size,
            ..Default::default()
        });
    }
    None
}

fn html_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&quot;", "\"")
        .replace("&#34;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

pub fn parse_part_note(desc: &str) -> Option<PartMeta> {
    let n = parse_file_note(desc)?;
    if n.kind != "part" || n.id.is_empty() || n.total < 1 || n.index < 1 {
        return None;
    }
    Some(PartMeta {
        group_id: n.id,
        name: n.name,
        index: n.index,
        total: n.total,
        size: n.size,
    })
}

pub fn parse_convert_note(desc: &str) -> Option<ConvertMeta> {
    let n = parse_file_note(desc)?;
    if n.kind != "convert" || n.name.is_empty() {
        return None;
    }
    Some(ConvertMeta {
        name: n.name,
        as_name: n.as_name,
        mode: n.mode,
        suffix: n.suffix,
        size: n.size,
    })
}

fn parse_legacy_part(desc: &str) -> Option<PartMeta> {
    const MARK: &str = "[lanzou-part]";
    let i = desc.find(MARK)?;
    let mut rest = desc[i + MARK.len()..].trim();
    if let Some(j) = rest.find(['\r', '\n']) {
        rest = &rest[..j];
    }
    let mut m = PartMeta::default();
    for field in rest.split_whitespace() {
        let Some((k, v)) = field.split_once('=') else {
            continue;
        };
        match k {
            "id" => m.group_id = v.into(),
            "name" => m.name = v.into(),
            "index" => m.index = v.parse().unwrap_or(0),
            "total" => m.total = v.parse().unwrap_or(0),
            "size" => m.size = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    if m.group_id.is_empty() || m.total < 1 || m.index < 1 {
        return None;
    }
    Some(m)
}

fn parse_legacy_convert(desc: &str) -> Option<ConvertMeta> {
    const MARK: &str = "[lanzou-convert]";
    let i = desc.find(MARK)?;
    let mut rest = desc[i + MARK.len()..].trim();
    if let Some(j) = rest.find(['\r', '\n']) {
        rest = &rest[..j];
    }
    let mut m = ConvertMeta::default();
    for field in rest.split_whitespace() {
        let Some((k, v)) = field.split_once('=') else {
            continue;
        };
        match k {
            "name" => m.name = v.into(),
            "as" => m.as_name = v.into(),
            "mode" => m.mode = v.into(),
            "suffix" => m.suffix = v.into(),
            "size" => m.size = v.parse().unwrap_or(0),
            _ => {}
        }
    }
    if m.name.is_empty() {
        return None;
    }
    Some(m)
}
