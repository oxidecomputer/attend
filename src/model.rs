use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::db::{self, RawEditor};

/// Resolved editor state: open files with cursor positions and terminal cwds.
#[derive(Debug, Serialize)]
pub struct EditorState {
    /// Open editor tabs with resolved line:col selections.
    pub files: Vec<FileEntry>,
    /// Working directories of active terminal tabs.
    pub terminals: Vec<String>,
}

/// An open file with its cursor/selection positions.
#[derive(Debug, Serialize)]
pub struct FileEntry {
    /// Absolute or cwd-relative file path.
    pub path: String,
    /// Cursor positions and selections in this file.
    pub selections: Vec<Selection>,
}

/// A 1-based line:col position in a file.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Position {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column (byte offset within the line).
    pub col: usize,
}

/// A selection range (or cursor when start == end).
#[derive(Debug, Serialize)]
pub struct Selection {
    /// Start of the selection.
    pub start: Position,
    /// End of the selection (equal to start for a cursor).
    pub end: Position,
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.line, self.col)
    }
}

impl fmt::Display for Selection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.start == self.end {
            write!(f, "{}", self.start)
        } else {
            write!(f, "{}-{}", self.start, self.end)
        }
    }
}

impl fmt::Display for FileEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)?;
        let positions: Vec<String> = self.selections.iter().map(|s| s.to_string()).collect();
        if !positions.is_empty() {
            write!(f, " {}", positions.join(","))?;
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
            write!(f, "{file}")?;
            first = false;
        }
        for terminal in &self.terminals {
            if !first {
                writeln!(f)?;
            }
            write!(f, "{terminal} $")?;
            first = false;
        }
        Ok(())
    }
}

/// Convert sorted, deduplicated byte offsets to (line, col) positions in a
/// single forward pass. Offsets past EOF map to the final position.
fn byte_offsets_to_positions(path: &Path, offsets: &[usize]) -> anyhow::Result<Vec<Position>> {
    let max_offset = match offsets.last() {
        Some(&o) => o,
        None => return Ok(Vec::new()),
    };

    let file = fs::File::open(path).context(format!("cannot open {}", path.display()))?;
    let mut reader = io::BufReader::new(file);
    let mut result = Vec::with_capacity(offsets.len());
    let mut line = 1;
    let mut col = 1;
    let mut cursor = 0;
    let mut offset_idx = 0;

    while cursor <= max_offset && offset_idx < offsets.len() {
        // Emit positions for any offsets at the current cursor
        while offset_idx < offsets.len() && offsets[offset_idx] <= cursor {
            result.push(Position { line, col });
            offset_idx += 1;
        }
        if offset_idx >= offsets.len() {
            break;
        }

        let buf = reader.fill_buf().context(format!("read error in {}", path.display()))?;
        if buf.is_empty() {
            break;
        }
        let need = offsets[offset_idx] - cursor;
        let n = buf.len().min(need);
        for &b in &buf[..n] {
            if b == b'\n' {
                line += 1;
                col = 1;
            } else {
                col += 1;
            }
        }
        cursor += n;
        reader.consume(n);
    }

    // Emit remaining offsets (at or past EOF)
    while offset_idx < offsets.len() {
        result.push(Position { line, col });
        offset_idx += 1;
    }

    Ok(result)
}

/// Make `path` relative to `cwd`, or return it unchanged if outside cwd.
fn relativize(path: &Path, cwd: Option<&Path>) -> String {
    let Some(cwd) = cwd else {
        return path.to_string_lossy().into_owned();
    };
    match path.strip_prefix(cwd) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => rel.to_string_lossy().into_owned(),
        Err(_) => path.to_string_lossy().into_owned(),
    }
}

/// Resolve raw byte-offset pairs to line:col selections by reading the file.
///
/// Deduplicates pairs, collects unique offsets for a single forward scan,
/// then reconstructs selections from the offset-to-position lookup.
fn resolve_selections(path: &Path, raw: &[(i64, i64)]) -> anyhow::Result<Vec<Selection>> {
    let mut seen: Vec<(i64, i64)> = raw.to_vec();
    seen.sort();
    seen.dedup();

    // Collect all unique offsets, sorted, for a single forward scan
    let mut all_offsets: Vec<usize> = seen
        .iter()
        .flat_map(|&(s, e)| [s as usize, e as usize])
        .collect();
    all_offsets.sort_unstable();
    all_offsets.dedup();

    let positions = byte_offsets_to_positions(path, &all_offsets)?;
    let lookup: std::collections::HashMap<usize, Position> =
        all_offsets.into_iter().zip(positions).collect();

    seen.iter()
        .map(|&(s, e)| {
            let start = lookup
                .get(&(s as usize))
                .cloned()
                .context(format!("missing offset {s} in lookup for {}", path.display()))?;
            let end = lookup
                .get(&(e as usize))
                .cloned()
                .context(format!("missing offset {e} in lookup for {}", path.display()))?;
            Ok(Selection { start, end })
        })
        .collect()
}

/// Build resolved editor state from raw DB rows: filter, group, resolve, relativize.
pub fn build_editor_state(
    raw_editors: Vec<RawEditor>,
    raw_terminals: Vec<PathBuf>,
    cwd: Option<&Path>,
) -> anyhow::Result<EditorState> {
    // Group by path, merging selections across panes/workspaces
    let mut files_map: BTreeMap<&Path, Vec<(i64, i64)>> = BTreeMap::new();
    for ed in &raw_editors {
        if let Some(cwd) = cwd
            && !ed.path.starts_with(cwd)
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
            resolve_selections(path, raw_sels)?
        };
        files.push(FileEntry {
            path: relativize(path, cwd),
            selections,
        });
    }

    let terminals: Vec<String> = raw_terminals
        .iter()
        .map(|p| relativize(p, cwd))
        .collect();

    Ok(EditorState { files, terminals })
}

/// Find the Zed DB, query editors, and build state for the given cwd.
/// Returns `None` if no DB is found or no files match.
pub fn get_editor_state(cwd: Option<&Path>) -> anyhow::Result<Option<EditorState>> {
    let db_path = match db::find_zed_db() {
        Some(p) => p,
        None => return Ok(None),
    };
    let result = db::query(&db_path)?;
    let state = build_editor_state(result.editors, result.terminals, cwd)?;
    if state.files.is_empty() && state.terminals.is_empty() {
        return Ok(None);
    }
    Ok(Some(state))
}
