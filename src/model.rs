use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io::{self, BufRead};
use std::path::Path;

use serde::Serialize;

use crate::db::{self, RawEditor};

#[derive(Debug, Serialize)]
pub struct EditorState {
    pub files: Vec<FileEntry>,
    pub terminals: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub selections: Vec<Selection>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Position {
    pub line: usize,
    pub col: usize,
}

#[derive(Debug, Serialize)]
pub struct Selection {
    pub start: Position,
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

/// Convert a sorted list of byte offsets to positions in a single forward scan.
/// Reads only up to the last offset from the file.
fn byte_offsets_to_positions(path: &str, offsets: &[usize]) -> Option<Vec<Position>> {
    let max_offset = match offsets.last() {
        Some(&o) => o,
        None => return Some(Vec::new()),
    };

    let file = fs::File::open(path).ok()?;
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

        let buf = reader.fill_buf().ok()?;
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

    Some(result)
}

fn relativize(path: &str, cwd: Option<&str>) -> String {
    let Some(cwd) = cwd else {
        return path.to_string();
    };
    let rel = if let Some(stripped) = path.strip_prefix(&format!("{cwd}/")) {
        stripped.to_string()
    } else if let Some(stripped) = path.strip_prefix(cwd) {
        stripped.trim_start_matches('/').to_string()
    } else {
        path.to_string()
    };
    if rel.is_empty() { ".".to_string() } else { rel }
}

pub fn build_editor_state(
    raw_editors: Vec<RawEditor>,
    raw_terminals: Vec<String>,
    cwd: Option<&Path>,
) -> EditorState {
    let cwd_str = cwd.map(|p| p.to_string_lossy().to_string());
    let cwd_ref = cwd_str.as_deref();

    // Group by path, merging selections across panes/workspaces
    let mut files_map: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    for ed in &raw_editors {
        if let Some(cwd) = cwd_ref
            && !ed.path.starts_with(cwd)
        {
            continue;
        }
        let entry = files_map.entry(ed.path.clone()).or_default();
        if let (Some(start), Some(end)) = (ed.sel_start, ed.sel_end) {
            entry.push((start, end));
        }
    }

    let mut files = Vec::new();
    for (path, selections) in &files_map {
        let mut sels = Vec::new();
        if !selections.is_empty() {
            let mut seen: Vec<(i64, i64)> = selections.clone();
            seen.sort();
            seen.dedup();

            let all_offsets: Vec<usize> = seen
                .iter()
                .flat_map(|&(s, e)| [s as usize, e as usize])
                .collect();

            if let Some(positions) = byte_offsets_to_positions(path, &all_offsets) {
                let mut pos_iter = positions.into_iter();
                for _ in &seen {
                    let start = pos_iter.next().unwrap();
                    let end = pos_iter.next().unwrap();
                    sels.push(Selection { start, end });
                }
            }
        }

        files.push(FileEntry {
            path: relativize(path, cwd_ref),
            selections: sels,
        });
    }

    let terminals: Vec<String> = raw_terminals
        .iter()
        .map(|p| relativize(p, cwd_ref))
        .collect();

    EditorState { files, terminals }
}

/// Find the Zed DB, query editors, and build state for the given cwd.
/// Returns `None` if no DB is found or no files match.
pub fn get_editor_state(cwd: Option<&Path>) -> anyhow::Result<Option<EditorState>> {
    let db_path = match db::find_zed_db() {
        Some(p) => p,
        None => return Ok(None),
    };
    let result = db::query(&db_path)?;
    let state = build_editor_state(result.editors, result.terminals, cwd);
    if state.files.is_empty() && state.terminals.is_empty() {
        return Ok(None);
    }
    Ok(Some(state))
}
