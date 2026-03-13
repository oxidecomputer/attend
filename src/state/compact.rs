//! Compact JSON output types for `attend --format json`.

use serde::Serialize;

use super::resolve;
use super::{EditorState, Position, Selection};

/// JSON representation of a file with cursors and selections split.
#[derive(Serialize)]
pub struct CompactFile {
    pub path: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cursors: Vec<Position>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub selections: Vec<Selection>,
}

/// Top-level JSON payload for `attend --format json`.
#[derive(Serialize)]
pub struct CompactPayload {
    pub files: Vec<CompactFile>,
}

impl CompactPayload {
    /// Build from an `EditorState`, splitting cursor-like selections from ranges.
    pub fn from_state(state: &EditorState) -> Self {
        let files = state
            .files
            .iter()
            .map(|entry| {
                let path = resolve::relativize(&entry.path, state.cwd.as_deref()).to_string();
                let (cursors, selections) = Selection::split(&entry.selections);
                CompactFile {
                    path,
                    cursors,
                    selections,
                }
            })
            .collect();
        CompactPayload { files }
    }
}
