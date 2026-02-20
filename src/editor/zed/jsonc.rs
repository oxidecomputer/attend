//! JSONC parsing utilities for Zed config files.
//!
//! Zed uses JSON with comments and trailing commas (JSONC). Editing is
//! performed via the `jsonc-parser` crate's CST, which preserves comments
//! and formatting across read-modify-write cycles.

use std::fs;
use std::path::{Path, PathBuf};

use jsonc_parser::ParseOptions;
use jsonc_parser::cst::{CstInputValue, CstRootNode};

/// A parsed JSONC file with a root array, supporting comment-preserving edits.
///
/// Reads the file as a CST (concrete syntax tree), provides serde_json::Value
/// access for semantic queries, and writes back preserving comments.
pub(super) struct JsoncArray {
    root: CstRootNode,
    path: PathBuf,
    modified: bool,
}

impl JsoncArray {
    /// Open a JSONC file. Returns an empty array if the file doesn't exist.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let text = if path.exists() {
            fs::read_to_string(path)?
        } else {
            "[\n]\n".to_string()
        };
        let root = CstRootNode::parse(&text, &ParseOptions::default())
            .map_err(|e| anyhow::anyhow!("JSONC parse error: {e}"))?;
        // Ensure root is an array.
        root.array_value_or_set();
        Ok(Self {
            root,
            path: path.to_path_buf(),
            modified: false,
        })
    }

    /// Get all elements as serde_json::Value for querying.
    pub fn elements(&self) -> Vec<serde_json::Value> {
        let Some(arr) = self.root.array_value() else {
            return Vec::new();
        };
        arr.elements()
            .iter()
            .filter_map(|node| node.to_serde_value())
            .collect()
    }

    /// Remove elements where the predicate returns false (like Vec::retain).
    ///
    /// Returns the number of elements removed.
    pub fn retain(&mut self, f: impl Fn(&serde_json::Value) -> bool) -> usize {
        let Some(arr) = self.root.array_value() else {
            return 0;
        };
        let to_remove: Vec<_> = arr
            .elements()
            .into_iter()
            .filter(|node| node.to_serde_value().is_none_or(|v| !f(&v)))
            .collect();
        let count = to_remove.len();
        for node in to_remove {
            node.remove();
        }
        if count > 0 {
            self.modified = true;
        }
        count
    }

    /// Append a serde_json::Value to the array.
    pub fn push(&mut self, value: serde_json::Value) {
        let arr = self.root.array_value_or_set();
        arr.append(serde_to_cst(value));
        self.modified = true;
    }

    /// Whether any modifications have been made.
    pub fn is_modified(&self) -> bool {
        self.modified
    }

    /// Write back to disk (atomic), preserving comments and formatting.
    pub fn save(&self) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut output = self.root.to_string();
        if !output.ends_with('\n') {
            output.push('\n');
        }
        crate::util::atomic_write_str(&self.path, &output)?;
        Ok(())
    }
}

/// Read a Zed JSONC config file as a JSON array, or empty vec if missing/invalid.
///
/// Use this for read-only access (e.g. health checks). For read-modify-write,
/// use [`JsoncArray`] to preserve comments.
pub(super) fn read_jsonc_array(path: &Path) -> Vec<serde_json::Value> {
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

/// Parse a JSONC string into any deserializable type.
pub(super) fn parse_jsonc<T: serde::de::DeserializeOwned>(input: &str) -> anyhow::Result<T> {
    let value = jsonc_parser::parse_to_serde_value(input, &ParseOptions::default())
        .map_err(|e| anyhow::anyhow!("JSONC parse error: {e}"))?
        .unwrap_or(serde_json::Value::Null);
    Ok(serde_json::from_value(value)?)
}

/// Convert a serde_json::Value to a CstInputValue for CST insertion.
fn serde_to_cst(value: serde_json::Value) -> CstInputValue {
    match value {
        serde_json::Value::Null => CstInputValue::Null,
        serde_json::Value::Bool(b) => CstInputValue::Bool(b),
        serde_json::Value::Number(n) => CstInputValue::Number(n.to_string()),
        serde_json::Value::String(s) => CstInputValue::String(s),
        serde_json::Value::Array(arr) => {
            CstInputValue::Array(arr.into_iter().map(serde_to_cst).collect())
        }
        serde_json::Value::Object(obj) => {
            CstInputValue::Object(obj.into_iter().map(|(k, v)| (k, serde_to_cst(v))).collect())
        }
    }
}
