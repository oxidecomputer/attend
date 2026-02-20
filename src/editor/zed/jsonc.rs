//! JSONC parsing utilities for Zed config files.
//!
//! Zed uses JSON with `//` line comments and trailing commas (JSONC).
//! These functions strip comments and trailing commas before handing
//! the content to `serde_json`.

use std::fs;

/// Read a Zed JSONC config file as a JSON array, or empty vec if missing/invalid.
pub(super) fn read_jsonc_array(path: &std::path::Path) -> Vec<serde_json::Value> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    match parse_jsonc(&content) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(path = %path.display(), "Failed to parse JSONC: {e}");
            Vec::new()
        }
    }
}

/// Write a JSON array to a config file with pretty formatting.
pub(super) fn write_json_array(
    path: &std::path::Path,
    items: &[serde_json::Value],
) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = serde_json::to_string_pretty(items)?;
    output.push('\n');
    fs::write(path, output)?;
    Ok(())
}

/// Parse a JSONC string (Zed's config format: `//` comments + trailing commas).
pub(super) fn parse_jsonc<T: serde::de::DeserializeOwned>(input: &str) -> serde_json::Result<T> {
    let stripped = strip_json_comments(input);
    let clean = strip_trailing_commas(&stripped);
    serde_json::from_str(&clean)
}

/// Strip `//` line comments from JSON content (Zed supports comments in JSON).
pub(super) fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        if let Some(idx) = find_line_comment(line) {
            out.push_str(&line[..idx]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Strip trailing commas before `]` and `}` (Zed allows trailing commas in JSONC).
pub(super) fn strip_trailing_commas(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut in_string = false;
    let mut escaped = false;

    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            out.push(bytes[i]);
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => {
                escaped = true;
                out.push(bytes[i]);
            }
            b'"' => {
                in_string = !in_string;
                out.push(bytes[i]);
            }
            b',' if !in_string => {
                // Look ahead past whitespace for ] or }
                let rest = &bytes[i + 1..];
                let next = rest.iter().find(|&&b| !b.is_ascii_whitespace());
                if next.is_some_and(|&b| b == b']' || b == b'}') {
                    continue; // skip trailing comma
                }
                out.push(bytes[i]);
            }
            _ => {
                out.push(bytes[i]);
            }
        }
    }

    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

/// Find the position of a `//` comment that's not inside a JSON string.
pub(super) fn find_line_comment(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;
    let bytes = line.as_bytes();

    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'/' if !in_string && i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}
