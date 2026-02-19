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
pub trait Editor: Sync {
    /// CLI name (e.g., "zed").
    fn name(&self) -> &'static str;

    /// Returns `Ok(None)` when the editor is not running or has no data.
    fn query(&self) -> anyhow::Result<Option<QueryResult>>;

    /// Filesystem paths to monitor for changes. When any file under these
    /// paths is modified, the backend should be re-queried. Returns an empty
    /// vec if filesystem notification is not supported.
    #[allow(dead_code)]
    fn watch_paths(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    /// Install dictation integration (task, keybinding, etc.).
    fn install_dictation(&self, _bin_cmd: &str) -> anyhow::Result<()> {
        anyhow::bail!("{} does not support dictation", self.name())
    }

    /// Remove dictation integration.
    fn uninstall_dictation(&self) -> anyhow::Result<()> {
        anyhow::bail!("{} does not support dictation removal", self.name())
    }

    /// Check the health of dictation integration.
    /// Returns a list of diagnostic warnings (empty = healthy).
    fn check_dictation(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// All registered editor backends.
pub const EDITORS: &[&'static dyn Editor] = &[
    &zed::Zed,
    // <-- Add new editors here
];

/// Look up an editor by CLI name.
pub fn editor_by_name(name: &str) -> Option<&'static dyn Editor> {
    EDITORS.iter().find(|e| e.name() == name).copied()
}

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
