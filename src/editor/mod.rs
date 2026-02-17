mod zed;
// <-- When adding an editor, add a module for it here

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

/// Raw editors and terminals returned from an editor backend.
pub struct QueryResult {
    /// Active editor tabs with byte-offset selections.
    pub editors: Vec<RawEditor>,
    /// Working directories of active terminal tabs.
    pub terminals: Vec<PathBuf>,
}

/// Query all active editors for current state, merging results.
pub fn query() -> anyhow::Result<Option<QueryResult>> {
    let mut editors = Vec::new();
    let mut terminals = Vec::new();

    if let Some(result) = zed::query()? {
        editors.extend(result.editors);
        terminals.extend(result.terminals);
    }
    // <-- Query future editors here

    if editors.is_empty() && terminals.is_empty() {
        return Ok(None);
    }
    Ok(Some(QueryResult { editors, terminals }))
}
