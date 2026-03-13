use serde::Serialize;

use crate::state::{Line, Position, Selection};

/// JSON representation of a group of selections with rendered content.
#[derive(Serialize)]
pub struct ViewGroup {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cursors: Vec<Position>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub selections: Vec<Selection>,
    pub first_line: Line,
    pub last_line: Line,
    pub content: String,
}

/// JSON representation of a single file's view output.
#[derive(Serialize)]
pub struct ViewFile {
    pub path: String,
    pub groups: Vec<ViewGroup>,
}

/// Top-level JSON payload for `attend view --format json`.
#[derive(Serialize)]
pub struct ViewPayload {
    pub files: Vec<ViewFile>,
}
