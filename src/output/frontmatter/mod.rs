//! Frontmatter rendering for note documents.
//!
//! The CLI passes a single comma-separated `key: value` spec via
//! `--frontmatter`. This module parses it and renders a YAML block
//! prefixed to the note body. All values are plain strings; dynamic data
//! (timestamps, paths, etc.) is computed by the caller (typically via
//! shell substitution) and embedded into the spec verbatim.
//!
//! Limitations: values cannot contain commas. Keys are trimmed and must
//! be non-empty. Duplicate keys: last occurrence wins.
//!
//! Writing the note is higher priority than rendering frontmatter:
//! a malformed spec yields `None` (or skips affected entries), and the
//! note is written without frontmatter.

/// Render a YAML frontmatter block ("---\n...\n---\n\n") from a spec
/// string of the form "key1: value1, key2: value2, ...". Returns `None`
/// if the spec produces no usable entries.
pub fn render_from_spec(spec: &str) -> Option<String> {
    let mut entries: Vec<(String, String)> = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let Some((k, v)) = part.split_once(':') else {
            continue;
        };
        let k = k.trim();
        if k.is_empty() {
            continue;
        }
        let v = v.trim();
        if let Some(pos) = entries.iter().position(|(ek, _)| ek == k) {
            entries[pos] = (k.to_string(), v.to_string());
        } else {
            entries.push((k.to_string(), v.to_string()));
        }
    }
    if entries.is_empty() {
        return None;
    }
    let mut out = String::from("---\n");
    for (k, v) in entries {
        out.push_str(&format!("{}: {}\n", k, yaml_scalar(&v)));
    }
    out.push_str("---\n\n");
    Some(out)
}

/// Render a string value as a YAML scalar, quoting when needed.
fn yaml_scalar(s: &str) -> String {
    let first = s.chars().next();
    let needs_quote = s.is_empty()
        || s.chars()
            .any(|c| matches!(c, ':' | '#' | '\n' | '\r' | '"' | '\''))
        || matches!(
            first,
            Some('-' | '?' | '*' | '&' | '|' | '>' | '!' | '%' | '@' | '`' | '[' | '{')
        )
        || s.starts_with(' ')
        || s.ends_with(' ');
    if needs_quote {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{}\"", escaped)
    } else {
        s.to_string()
    }
}
