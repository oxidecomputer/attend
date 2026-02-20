use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::{fs, io};

use camino::{Utf8Path, Utf8PathBuf};
use serde::{Deserialize, Serialize};

use crate::editor::{self, RawEditor};
use crate::util::atomic_write;

/// Return the platform cache directory for attend.
pub fn cache_dir() -> Option<Utf8PathBuf> {
    let dir = dirs::cache_dir()?;
    let dir = Utf8PathBuf::try_from(dir).ok()?;
    Some(dir.join("attend"))
}

/// Path to the file that identifies the currently attending session.
pub fn listening_path() -> Option<Utf8PathBuf> {
    Some(cache_dir()?.join("listening"))
}

/// Read the session ID of the currently attending session, if any.
pub fn listening_session() -> Option<String> {
    std::fs::read_to_string(listening_path()?)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Path to the installed version/components file.
pub(crate) fn version_path() -> Option<Utf8PathBuf> {
    Some(cache_dir()?.join("version.json"))
}

/// Metadata about the most recent hook install.
#[derive(Serialize, Deserialize, Debug)]
pub(crate) struct InstallMeta {
    pub version: String,
    pub agents: Vec<String>,
    pub editors: Vec<String>,
    pub dev: bool,
}

/// Read the install metadata, if any.
pub(crate) fn installed_meta() -> Option<InstallMeta> {
    let path = version_path()?;
    let content = fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Save install metadata after a successful hook install.
pub(crate) fn save_install_meta(meta: &InstallMeta) {
    let Some(path) = version_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = atomic_write(&path, |file| {
        serde_json::to_writer_pretty(io::BufWriter::new(file), meta).map_err(io::Error::other)
    }) {
        tracing::warn!("Failed to save install metadata: {e}");
    }
}

/// Path to the shared ordering cache.
fn shared_cache_path() -> Option<Utf8PathBuf> {
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
    pub cwd: Option<Utf8PathBuf>,
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
    pub path: Utf8PathBuf,
    /// Cursor positions and selections in this file.
    pub selections: Vec<Selection>,
}

impl fmt::Display for FileEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let path = self.path.as_str();
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
    pub fn current(
        cwd: Option<&Utf8Path>,
        include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Option<Self>> {
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
        if let Some(parent) = cp.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            tracing::warn!(path = %parent, "Failed to create cache directory: {e}");
            return;
        }
        if let Err(e) = atomic_write(&cp, |file| {
            serde_json::to_writer(io::BufWriter::new(file), self).map_err(io::Error::other)
        }) {
            tracing::warn!(path = %cp, "Failed to write cache: {e}");
        }
    }

    /// Build resolved editor state from raw editor rows: filter, group, resolve.
    ///
    /// Files are included if they are under `cwd` or any of `include_dirs`.
    /// Pass `None` for `cwd` to include all files (no filtering).
    pub fn build(
        raw_editors: Vec<RawEditor>,
        cwd: Option<&Utf8Path>,
        include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Self> {
        // Convert RawEditor paths to UTF-8, skipping non-UTF-8 entries.
        let utf8_editors: Vec<(Utf8PathBuf, Option<i64>, Option<i64>)> = raw_editors
            .into_iter()
            .filter_map(|ed| {
                Utf8PathBuf::try_from(ed.path)
                    .ok()
                    .map(|p| (p, ed.sel_start, ed.sel_end))
            })
            .collect();

        // Group by path, merging selections across panes/workspaces
        let mut files_map: BTreeMap<&Utf8Path, Vec<(i64, i64)>> = BTreeMap::new();
        for (path, sel_start, sel_end) in &utf8_editors {
            if let Some(cwd) = cwd
                && !path.starts_with(cwd)
                && !include_dirs.iter().any(|d| path.starts_with(d))
            {
                continue;
            }
            let entry = files_map.entry(path.as_path()).or_default();
            if let (Some(start), Some(end)) = (sel_start, sel_end) {
                entry.push((*start, *end));
            }
        }

        let mut files = Vec::new();
        for (path, raw_sels) in &files_map {
            let selections = if raw_sels.is_empty() {
                Vec::new()
            } else {
                Selection::resolve(path.as_std_path(), raw_sels)?
            };
            files.push(FileEntry {
                path: path.to_path_buf(),
                selections,
            });
        }

        Ok(EditorState {
            files,
            cwd: cwd.map(Utf8Path::to_path_buf),
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
        let prev_map: HashMap<&Utf8Path, (usize, &Vec<Selection>)> = previous
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
