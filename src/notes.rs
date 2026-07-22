//! Unified JSON file notes stored in Lanzou descriptions (schema v1 only).
//!
//! ```json
//! {"v":1,"kind":"raw","name":"a.txt","as":"a.txt","size":12}
//! {"v":1,"kind":"convert","name":"a.dex","as":"a.dex.zip","mode":"zip","suffix":"zip","size":20}
//! {"v":2,"kind":"part","id":"...","name":"big.bin","as":"big_s001.zip","index":1,"total":3,"size":1048576,
//!  "nextId":"123","nextUrl":"https://.../xxx","npwd":"ab12"}
//!
//! Part chain: v1 `next` is always a file id; v2 uses `nextId` + `nextUrl` (+ `npwd`).
//! Never treat v1 `next` as a URL.
//! ```

use serde::{Deserialize, Serialize};

/// Note schema version.
pub const NOTE_VERSION: u32 = 2;

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
    /// v1 only: next part remote file id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub next: String,
    /// v2: next part file id.
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "nextId")]
    pub next_id: String,
    /// v2: next part share URL.
    #[serde(default, skip_serializing_if = "String::is_empty", rename = "nextUrl")]
    pub next_url: String,
    /// Password for next_url (kind=part).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub npwd: String,
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
    next_file_id: &str,
    next_share_url: &str,
    next_pwd: &str,
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
        next_id: next_file_id.into(),
        next_url: next_share_url.into(),
        npwd: next_pwd.into(),
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
    pub next_id: String,
    pub next_url: String,
    pub npwd: String,
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
    let desc = clean_share_desc(desc);
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
    if n.kind == "part" {
        n = normalize_part_note(n);
    }
    Some(n)
}

pub fn strip_html_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        if c == '<' {
            in_tag = true;
            continue;
        }
        if in_tag {
            if c == '>' {
                in_tag = false;
            }
            continue;
        }
        out.push(c);
    }
    out
}

pub fn clean_share_desc(s: &str) -> String {
    html_unescape(&strip_html_tags(s.trim()))
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


/// v1 "next" is always a file id → next_id. Never map next → next_url.
fn normalize_part_note(mut n: FileNote) -> FileNote {
    if n.next_id.is_empty() && !n.next.is_empty() {
        n.next_id = n.next.clone();
    }
    n
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
        next_id: n.next_id,
        next_url: n.next_url,
        npwd: n.npwd,
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


/// Pull the "文件描述" field from a share page HTML (often entity-encoded JSON).
pub fn extract_share_description(html: &str) -> String {
    if html.is_empty() {
        return String::new();
    }
    const LABEL: &str = "文件描述";
    if let Some(i) = html.find(LABEL) {
        let mut rest = &html[i + LABEL.len()..];
        rest = rest.trim_start_matches(['：', ':', ' ', '\t', '\r', '\n']);
        loop {
            rest = rest.trim_start();
            if rest.starts_with('<') {
                if let Some(j) = rest.find('>') {
                    rest = &rest[j + 1..];
                    continue;
                }
            }
            break;
        }
        if let Some(k) = rest.find('{') {
            let mut depth = 0i32;
            for (p, ch) in rest[k..].char_indices() {
                match ch {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            return rest[k..=k + p].trim().to_string();
                        }
                    }
                    _ => {}
                }
            }
        }
        for sep in ["</td>", "<td", "<div"] {
            if let Some(j) = rest.find(sep) {
                let cand = rest[..j]
                    .replace("<br>", "")
                    .replace("<br/>", "")
                    .replace("<br />", "");
                let cand = cand.trim();
                if !cand.is_empty() {
                    return cand.to_string();
                }
            }
        }
    }
    for marker in ["\"kind\"", "&quot;kind&quot;", "\"v\":1", "&quot;v&quot;:1"] {
        if let Some(k) = html.find(marker) {
            if let Some(start) = html[..k + 1].rfind('{') {
                let mut depth = 0i32;
                for (p, ch) in html[start..].char_indices() {
                    match ch {
                        '{' => depth += 1,
                        '}' => {
                            depth -= 1;
                            if depth == 0 {
                                return html[start..=start + p].trim().to_string();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
    String::new()
}
