//! Alibaba ESA / WAF style `acw_sc__v2` cookie pure computation.

use crate::error::{Error, Result};
use regex::Regex;
use std::sync::OnceLock;

/// Fixed mask recovered from challenge script (stable for current site family).
pub const MASK: &str = "3000176000856006061501533003690027800375";

/// Fixed 1-based permutation table from challenge script.
pub const PERM: [u8; 40] = [
    0x0F, 0x23, 0x1D, 0x18, 0x21, 0x10, 0x01, 0x26, 0x0A, 0x09, 0x13, 0x1F, 0x28, 0x1B, 0x16,
    0x17, 0x19, 0x0D, 0x06, 0x0B, 0x27, 0x12, 0x14, 0x08, 0x0E, 0x15, 0x20, 0x1A, 0x02, 0x1E,
    0x07, 0x04, 0x11, 0x05, 0x03, 0x1C, 0x22, 0x25, 0x0C, 0x24,
];

fn arg1_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"var\s+arg1\s*=\s*'([0-9A-Fa-f]+)'").unwrap())
}

/// Whether HTML looks like an acw challenge page.
pub fn is_acw_challenge(html: &str) -> bool {
    html.contains("var arg1=") && html.contains("acw_sc__v2")
}

/// Extract `arg1` from challenge HTML.
pub fn extract_arg1(html: &str) -> Option<String> {
    arg1_re()
        .captures(html)
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
}

/// Compute `acw_sc__v2` from server-issued `arg1`.
pub fn calc_acw_sc_v2(arg1: &str) -> Result<String> {
    calc_acw_sc_v2_with(arg1, MASK, &PERM)
}

/// Compute with explicit mask/perm (for tests / future rotation).
pub fn calc_acw_sc_v2_with(arg1: &str, mask: &str, perm: &[u8]) -> Result<String> {
    if arg1.len() != perm.len() {
        return Err(Error::Acw(format!(
            "arg1 length {} != perm length {}",
            arg1.len(),
            perm.len()
        )));
    }
    let chars: Vec<char> = arg1.chars().collect();
    let mut q = vec!['\0'; perm.len()];
    for (x, ch) in chars.iter().enumerate() {
        let pos = (x + 1) as u8;
        for (z, &p) in perm.iter().enumerate() {
            if p == pos {
                q[z] = *ch;
            }
        }
    }
    let u: String = q.into_iter().collect();
    let mut out = String::with_capacity(mask.len());
    let limit = u.len().min(mask.len());
    let mut i = 0;
    while i + 1 < limit {
        let left = u8::from_str_radix(&u[i..i + 2], 16).map_err(|e| Error::Acw(e.to_string()))?;
        let right =
            u8::from_str_radix(&mask[i..i + 2], 16).map_err(|e| Error::Acw(e.to_string()))?;
        out.push_str(&format!("{:02x}", left ^ right));
        i += 2;
    }
    Ok(out)
}

/// Return `(arg1, acw_sc__v2)` from challenge HTML.
pub fn solve_from_html(html: &str) -> Result<(String, String)> {
    let arg1 = extract_arg1(html).ok_or_else(|| Error::Acw("HTML has no arg1".into()))?;
    let token = calc_acw_sc_v2(&arg1)?;
    Ok((arg1, token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_sample() {
        let arg1 = "5D8776F79D6F46531729E68EEC1B548C74AB4569";
        let v = calc_acw_sc_v2(arg1).unwrap();
        assert_eq!(v, "6a5e6435d9adf16ee27366c45ecfbdf7300b478e");
    }
}
