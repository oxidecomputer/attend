//! JSONC parsing utilities for Zed config files.
//!
//! Zed uses JSON with comments and trailing commas (JSONC). Parsing is
//! delegated to the `jsonc-parser` crate; writing emits standard JSON
//! (comments are not preserved on write).

use std::fs;

use jsonc_parser::ParseOptions;

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
    crate::util::atomic_write_str(path, &output)?;
    Ok(())
}

/// Parse a JSONC string into any deserializable type.
pub(super) fn parse_jsonc<T: serde::de::DeserializeOwned>(input: &str) -> anyhow::Result<T> {
    let value = jsonc_parser::parse_to_serde_value(input, &ParseOptions::default())
        .map_err(|e| anyhow::anyhow!("JSONC parse error: {e}"))?
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::from_value(value)?)
}
