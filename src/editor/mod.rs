mod zed;
// <-- Add new editor modules here

use std::path::PathBuf;

/// A row from the editor before offset resolution.
pub struct RawEditor {
    /// Absolute file path.
    pub path: PathBuf,
    /// Byte offset of selection start, if any.
    pub sel_start: Option<i64>,
    /// Byte offset of selection end, if any.
    pub sel_end: Option<i64>,
}

/// Raw editors returned from an editor backend.
pub struct QueryResult {
    /// Active editor tabs with byte-offset selections.
    pub editors: Vec<RawEditor>,
}

/// A backend that can query an editor for open files.
pub trait Editor {
    /// Returns `Ok(None)` when the editor is not running or has no data.
    fn query(&self) -> anyhow::Result<Option<QueryResult>>;

    /// Filesystem paths to monitor for changes. When any file under these
    /// paths is modified, the backend should be re-queried. Returns an empty
    /// vec if filesystem notification is not supported.
    #[allow(dead_code)]
    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }
}

/// All registered editor backends.
const EDITORS: &'static [&'static dyn Editor] = &[
    &zed::Zed,
    // <-- Add new editors here
];

/// Collect all filesystem watch paths from every registered backend.
#[allow(dead_code)]
pub fn all_watch_paths() -> Vec<PathBuf> {
    EDITORS.iter().flat_map(|e| e.watch_paths()).collect()
}

/// Query all active editors for current state, merging results.
pub fn query() -> anyhow::Result<Option<QueryResult>> {
    let mut editors = Vec::new();

    for backend in EDITORS {
        if let Some(result) = backend.query()? {
            editors.extend(result.editors);
        }
    }

    if editors.is_empty() {
        return Ok(None);
    }

    Ok(Some(QueryResult { editors }))
}
