use serde::Serialize;

use crate::state::resolve::relativize;
use crate::state::{EditorState, Line, Position, Selection};

/// Return the current UTC time as an ISO 8601 string (e.g. `2026-02-18T15:30:45Z`).
pub fn utc_now() -> String {
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    unsafe { libc::gettimeofday(&mut tv, std::ptr::null_mut()) };
    let time = tv.tv_sec;
    let tm = unsafe { *libc::gmtime(&time) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
    )
}

/// Wrapper that adds a `timestamp` field to any serializable payload.
#[derive(Serialize)]
pub struct Timestamped<T: Serialize> {
    pub timestamp: String,
    #[serde(flatten)]
    pub inner: T,
}

impl<T: Serialize> Timestamped<T> {
    /// Wrap a payload with the current UTC timestamp.
    pub fn now(inner: T) -> Self {
        Self {
            timestamp: utc_now(),
            inner,
        }
    }
}

// ---------------------------------------------------------------------------
// Compact JSON types (attend --format json)
// ---------------------------------------------------------------------------

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
                let path = relativize(&entry.path, state.cwd.as_deref()).to_string();
                let (cursors, selections) = split_selections(&entry.selections);
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

// ---------------------------------------------------------------------------
// View JSON types (attend view --format json)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Split a list of selections into cursor positions and range selections.
pub fn split_selections(sels: &[Selection]) -> (Vec<Position>, Vec<Selection>) {
    let mut cursors = Vec::new();
    let mut selections = Vec::new();
    for sel in sels {
        if sel.is_cursor_like() {
            cursors.push(sel.start);
        } else {
            selections.push(*sel);
        }
    }
    (cursors, selections)
}
