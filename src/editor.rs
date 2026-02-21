mod zed;
// <-- Add new editor modules here

use std::path::PathBuf;

/// All registered editor backends.
pub const EDITORS: &[&'static dyn Editor] = &[
    &zed::Zed,
    // <-- Add new editors here
];

/// A row from the editor before offset resolution.
///
/// Backends currently provide byte offsets for selections; future backends
/// may provide line:col positions directly. The normalization to line:col
/// happens in `Selection::resolve()` during `EditorState::build()`.
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
pub trait Editor: Sync {
    /// CLI name (e.g., "zed").
    fn name(&self) -> &'static str;

    /// Returns `Ok(None)` when the editor is not running or has no data.
    fn query(&self) -> anyhow::Result<Option<QueryResult>>;

    /// Install narration integration (task, keybinding, etc.).
    fn install_narration(&self, _bin_cmd: &str) -> anyhow::Result<()> {
        anyhow::bail!("{} does not support narration", self.name())
    }

    /// Remove narration integration.
    fn uninstall_narration(&self) -> anyhow::Result<()> {
        anyhow::bail!("{} does not support narration removal", self.name())
    }

    /// Check the health of narration integration.
    /// Returns a list of diagnostic warnings (empty = healthy).
    fn check_narration(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// Look up an editor by CLI name.
pub fn editor_by_name(name: &str) -> Option<&'static dyn Editor> {
    EDITORS.iter().find(|e| e.name() == name).copied()
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
