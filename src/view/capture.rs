use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::state::resolve::relativize;
use crate::state::{FileEntry, Selection};

use super::annotate::{self, Group};
use super::{Extent, LanguageCache};

/// Resolve a `FileEntry` path to an absolute path.
///
/// If the path is already absolute it is returned as-is. Otherwise it is
/// joined to `cwd` (or the process working directory when `cwd` is `None`).
pub(super) fn resolve_abs_path(
    path: &Utf8Path,
    cwd: Option<&Utf8Path>,
) -> anyhow::Result<Utf8PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        let base = match cwd {
            Some(c) => c.to_path_buf(),
            None => Utf8PathBuf::try_from(std::env::current_dir()?).map_err(|e| {
                anyhow::anyhow!(
                    "non-UTF-8 working directory: {}",
                    e.into_path_buf().display()
                )
            })?,
        };
        Ok(base.join(path))
    }
}

/// A region of a file captured from an editor snapshot.
///
/// Stores raw (untrimmed, un-annotated) file content plus the selection
/// positions that were active at capture time. Marker annotation (⟦⟧❘) is
/// deferred to [`super::apply_markers`] at render time.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapturedRegion {
    /// Display path of the file (relative or absolute).
    pub path: String,
    /// Raw untrimmed lines for this region, joined with newlines.
    pub content: String,
    /// 1-based line number of the first line in `content`.
    pub first_line: u32,
    /// Absolute file positions of selections/cursors within this region.
    pub selections: Vec<Selection>,
    /// Programming language detected from the file path (e.g. "rust", "python").
    /// Uses GFM-compatible identifiers. `None` when detection fails, the file
    /// type is unknown, or the language lacks GFM syntax highlighting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

/// Capture raw file regions from editor entries, parallel to [`super::render_json`]
/// but without baking in selection markers.
///
/// Returns one [`CapturedRegion`] per selection group (context-merged range).
pub fn capture_regions(
    entries: &[FileEntry],
    cwd: Option<&Utf8Path>,
    extent: Extent,
    lang_cache: &mut LanguageCache,
) -> anyhow::Result<Vec<CapturedRegion>> {
    let mut regions = Vec::new();

    for entry in entries {
        let abs_path = resolve_abs_path(&entry.path, cwd)?;

        let display_path = relativize(&abs_path, cwd).to_string();

        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        // Detect language once per file (cached across captures).
        let language = lang_cache.detect(&abs_path);

        let groups = Group::compute(&entry.selections, lines.len(), extent);

        for group in &groups {
            let raw = annotate::raw_line_range(&lines, group.first_line, group.last_line);
            let sels = group.sels.iter().map(|s| **s).collect();
            regions.push(CapturedRegion {
                path: display_path.clone(),
                content: raw,
                first_line: group.first_line.get() as u32,
                selections: sels,
                language: language.clone(),
            });
        }
    }

    Ok(regions)
}
