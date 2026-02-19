use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};
use std::{fs, io};

use serde::{Deserialize, Serialize};

use crate::editor::{self, RawEditor};

/// Write to a file atomically by writing to a temporary sibling first.
///
/// Creates `<path>.tmp`, calls the writer closure, then renames to `<path>`.
/// This prevents readers from seeing partially-written files.
pub(crate) fn atomic_write(
    path: &Path,
    f: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    let mut file = fs::File::create(&tmp)?;
    match f(&mut file) {
        Ok(()) => fs::rename(&tmp, path),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Return the platform cache directory for attend.
pub fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("attend"))
}

/// Path to the file that identifies the currently attending session.
pub fn listening_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("listening"))
}

/// Read the session ID of the currently attending session, if any.
pub fn listening_session() -> Option<String> {
    std::fs::read_to_string(listening_path()?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Path to the shared ordering cache.
fn shared_cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("latest.json"))
}

/// Core types (Line, Col, Position, Selection) and byte-offset resolution.
pub(crate) mod resolve;
pub use resolve::{Col, Line, Position, Selection};

#[cfg(test)]
mod tests;

/// Resolved editor state: open files with cursor positions.
#[derive(Debug, Default, Eq, Serialize, Deserialize)]
pub struct EditorState {
    /// Open editor tabs with resolved line:col selections.
    pub files: Vec<FileEntry>,
    /// Working directory, used by Display for relativization.
    #[serde(skip)]
    pub cwd: Option<PathBuf>,
}

impl PartialEq for EditorState {
    fn eq(&self, other: &Self) -> bool {
        self.files == other.files
    }
}

/// An open file with its cursor/selection positions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileEntry {
    /// Absolute file path.
    pub path: PathBuf,
    /// Cursor positions and selections in this file.
    pub selections: Vec<Selection>,
}

impl fmt::Display for FileEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.display().to_string();
        if path.contains(' ') {
            write!(f, "\"{path}\"")?;
        } else {
            write!(f, "{path}")?;
        }
        for (i, sel) in self.selections.iter().enumerate() {
            if i == 0 {
                write!(f, " ")?;
            } else {
                write!(f, ", ")?;
            }
            write!(f, "{sel}")?;
        }
        Ok(())
    }
}

impl fmt::Display for EditorState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        for file in &self.files {
            if !first {
                writeln!(f)?;
            }
            let path = resolve::relativize(&file.path, self.cwd.as_deref());
            if path.contains(' ') {
                write!(f, "\"{path}\"")?;
            } else {
                write!(f, "{path}")?;
            }
            for (i, sel) in file.selections.iter().enumerate() {
                if i == 0 {
                    write!(f, " ")?;
                } else {
                    write!(f, ", ")?;
                }
                write!(f, "{sel}")?;
            }
            first = false;
        }
        Ok(())
    }
}

impl EditorState {
    /// Load current editor state from all active editors, reordering
    /// by recency relative to the shared cache, and update the cache.
    pub fn current(cwd: Option<&Path>, include_dirs: &[PathBuf]) -> anyhow::Result<Option<Self>> {
        let result = match editor::query()? {
            Some(r) => r,
            None => return Ok(None),
        };
        let mut state = Self::build(result.editors, cwd, include_dirs)?;
        if state.files.is_empty() {
            return Ok(None);
        }
        let previous = Self::load_cached().unwrap_or_default();
        state.reorder_relative_to(&previous);
        state.save_cache();
        Ok(Some(state))
    }

    /// Load the shared (cross-session) cached editor state for recency ordering.
    fn load_cached() -> Option<Self> {
        let cp = shared_cache_path()?;
        let s = fs::read_to_string(&cp).ok()?;
        serde_json::from_str(&s).ok()
    }

    /// Save to the shared cache so all sessions benefit from fresh ordering.
    fn save_cache(&self) {
        let Some(cp) = shared_cache_path() else {
            return;
        };
        if let Some(parent) = cp.parent() {
            if let Err(e) = fs::create_dir_all(parent) {
                tracing::warn!(path = %parent.display(), "Failed to create cache directory: {e}");
                return;
            }
        }
        if let Err(e) = atomic_write(&cp, |file| {
            serde_json::to_writer(io::BufWriter::new(file), self)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
        }) {
            tracing::warn!(path = %cp.display(), "Failed to write cache: {e}");
        }
    }

    /// Build resolved editor state from raw editor rows: filter, group, resolve.
    ///
    /// Files are included if they are under `cwd` or any of `include_dirs`.
    /// Pass `None` for `cwd` to include all files (no filtering).
    pub fn build(
        raw_editors: Vec<RawEditor>,
        cwd: Option<&Path>,
        include_dirs: &[PathBuf],
    ) -> anyhow::Result<Self> {
        // Group by path, merging selections across panes/workspaces
        let mut files_map: BTreeMap<&Path, Vec<(i64, i64)>> = BTreeMap::new();
        for ed in &raw_editors {
            if let Some(cwd) = cwd
                && !ed.path.starts_with(cwd)
                && !include_dirs.iter().any(|d| ed.path.starts_with(d))
            {
                continue;
            }
            let entry = files_map.entry(&ed.path).or_default();
            if let (Some(start), Some(end)) = (ed.sel_start, ed.sel_end) {
                entry.push((start, end));
            }
        }

        let mut files = Vec::new();
        for (path, raw_sels) in &files_map {
            let selections = if raw_sels.is_empty() {
                Vec::new()
            } else {
                Selection::resolve(path, raw_sels)?
            };
            files.push(FileEntry {
                path: path.to_path_buf(),
                selections,
            });
        }

        Ok(EditorState {
            files,
            cwd: cwd.map(Path::to_path_buf),
        })
    }

    /// Reorder files and selections so recently changed items appear first.
    ///
    /// - Files not present in the previous state (new) or with changed selections
    ///   move to the front, preserving their relative (alphabetical) order.
    /// - Unchanged files retain their position from the previous (cached) order.
    /// - Within a touched file, new/changed selections come first; unchanged
    ///   selections keep their cached order.
    pub fn reorder_relative_to(&mut self, previous: &EditorState) {
        // Map previous path → (index, &selections)
        let prev_map: HashMap<&Path, (usize, &Vec<Selection>)> = previous
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| (f.path.as_path(), (i, &f.selections)))
            .collect();

        // Classify each file as touched (None) or unchanged (Some(cached_index))
        let mut tagged: Vec<(Option<usize>, FileEntry)> = Vec::with_capacity(self.files.len());

        for file in self.files.drain(..) {
            match prev_map.get(file.path.as_path()) {
                None => {
                    // New file → touched
                    tagged.push((None, file));
                }
                Some(&(cached_idx, prev_sels)) => {
                    if file.selections == *prev_sels {
                        // Unchanged → keep cached position
                        tagged.push((Some(cached_idx), file));
                    } else {
                        // Changed selections → touched; reorder selections
                        let mut new_sels = Vec::with_capacity(file.selections.len());
                        let mut unchanged_sels: Vec<(usize, Selection)> = Vec::new();

                        // Build a set of (cached_index) for previous selections
                        let prev_sel_indices: HashMap<(&Position, &Position), usize> = prev_sels
                            .iter()
                            .enumerate()
                            .map(|(i, s)| ((&s.start, &s.end), i))
                            .collect();

                        for sel in file.selections {
                            match prev_sel_indices.get(&(&sel.start, &sel.end)) {
                                Some(&idx) => unchanged_sels.push((idx, sel)),
                                None => new_sels.push(sel),
                            }
                        }
                        unchanged_sels.sort_by_key(|(idx, _)| *idx);
                        new_sels.extend(unchanged_sels.into_iter().map(|(_, s)| s));

                        tagged.push((
                            None,
                            FileEntry {
                                path: file.path,
                                selections: new_sels,
                            },
                        ));
                    }
                }
            }
        }

        // Stable partition: touched (None) first, then unchanged sorted by cached index.
        // touched files keep their relative order (alphabetical from build).
        tagged.sort_by_key(|(cached_idx, _)| match cached_idx {
            None => (0, 0),
            Some(idx) => (1, *idx),
        });

        self.files = tagged.into_iter().map(|(_, f)| f).collect();
    }
}
