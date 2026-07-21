//! Unified JSON file notes stored in Lanzou descriptions (schema v1 only).
//!
//! ```json
//! {"v":1,"kind":"raw","name":"a.txt","as":"a.txt","size":12}
//! {"v":1,"kind":"convert","name":"a.dex","as":"a.dex.zip","mode":"zip","suffix":"zip","size":20}
//! {"v":1,"kind":"part","id":"...","name":"big.bin","as":"big_part001.zip","index":1,"total":3,"size":1048576}
//! ```

use serde::{Deserialize, Serialize};

/// Note schema version.
pub const NOTE_VERSION: u32 = 1;

/// Unified note schema written to file descriptions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FileNote {
    #[serde(default = "default_v")]
    pub v: u32,
    pub kind: String, // raw | convert | part
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
    NOTE_VERSION
}
fn is_zero_usize(n: &usize) -> bool {
    *n == 0
}
fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}

/// JSON note for an upload that did not convert the suffix.
pub fn format_raw_note(orig_name: &str, upload_name: &str, size: u64) -> String {
    let as_name = if upload_name.is_empty() {
        orig_name
    } else {
        upload_name
    };
    serde_json::to_string(&FileNote {
        v: NOTE_VERSION,
        kind: "raw".into(),
        name: orig_name.into(),
        as_name: as_name.into(),
        size,
        ..Default::default()
    })
    .unwrap_or_default()
}

/// JSON convert note.
pub fn format_convert_note(
    orig_name: &str,
    upload_name: &str,
    mode: &str,
    suffix: &str,
    size: u64,
) -> String {
    serde_json::to_string(&FileNote {
        v: NOTE_VERSION,
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

/// JSON part note.
pub fn format_part_note(
    group_id: &str,
    orig_name: &str,
    upload_name: &str,
    index: usize,
    total: usize,
    size: u64,
) -> String {
    serde_json::to_string(&FileNote {
        v: NOTE_VERSION,
        kind: "part".into(),
        id: group_id.into(),
        name: orig_name.into(),
        as_name: upload_name.into(),
        index,
        total,
        size,
        ..Default::default()
    })
    .unwrap_or_default()
}

/// Parsed part metadata.
#[derive(Debug, Clone, Default)]
pub struct PartMeta {
    pub group_id: String,
    pub name: String,
    pub as_name: String,
    pub index: usize,
    pub total: usize,
    pub size: u64,
}

/// Parsed convert/raw metadata.
#[derive(Debug, Clone, Default)]
pub struct ConvertMeta {
    pub name: String,
    pub as_name: String,
    pub mode: String,
    pub suffix: String,
    pub size: u64,
    pub raw: bool,
}

/// Parse a v1 JSON note only (after HTML unescape).
pub fn parse_file_note(desc: &str) -> Option<FileNote> {
    let desc = html_unescape(desc.trim());
    if desc.is_empty() {
        return None;
    }
    let i = desc.find('{')?;
    let j = desc.rfind('}')?;
    if j <= i {
        return None;
    }
    let mut n: FileNote = serde_json::from_str(&desc[i..=j]).ok()?;
    match n.kind.as_str() {
        "raw" | "convert" | "part" => {}
        _ => return None,
    }
    if n.v == 0 {
        n.v = NOTE_VERSION;
    }
    Some(n)
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
        as_name: n.as_name,
        index: n.index,
        total: n.total,
        size: n.size,
    })
}

/// Convert or raw notes (raw treated as same-name convert for resolution).
pub fn parse_convert_note(desc: &str) -> Option<ConvertMeta> {
    let n = parse_file_note(desc)?;
    match n.kind.as_str() {
        "convert" => {
            if n.name.is_empty() {
                return None;
            }
            Some(ConvertMeta {
                name: n.name,
                as_name: n.as_name,
                mode: n.mode,
                suffix: n.suffix,
                size: n.size,
                raw: false,
            })
        }
        "raw" => {
            if n.name.is_empty() {
                return None;
            }
            let as_name = if n.as_name.is_empty() {
                n.name.clone()
            } else {
                n.as_name
            };
            Some(ConvertMeta {
                name: n.name,
                as_name,
                size: n.size,
                raw: true,
                ..Default::default()
            })
        }
        _ => None,
    }
}
