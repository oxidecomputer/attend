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
}

#[derive(Debug, Serialize)]
pub struct FileEntry {
    pub path: String,
    pub cursors: Vec<Position>,
    pub selections: Vec<Selection>,
}

#[derive(Debug, Clone, Serialize)]
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

impl fmt::Display for FileEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.path)?;
        let positions: Vec<String> = self
            .cursors
            .iter()
            .map(|c| c.to_string())
            .chain(
                self.selections
                    .iter()
                    .map(|s| format!("{}-{}", s.start, s.end)),
            )
            .collect();
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

pub fn build_editor_state(raw_editors: Vec<RawEditor>, cwd: Option<&Path>) -> EditorState {
    let cwd_str = cwd.map(|p| p.to_string_lossy().to_string());

    // Group by path, merging selections across panes/workspaces
    let mut files_map: BTreeMap<String, Vec<(i64, i64)>> = BTreeMap::new();
    for ed in &raw_editors {
        if let Some(ref cwd) = cwd_str
            && !ed.path.starts_with(cwd.as_str())
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
        let rel_path = if let Some(ref cwd) = cwd_str {
            if let Some(stripped) = path.strip_prefix(&format!("{cwd}/")) {
                stripped.to_string()
            } else if let Some(stripped) = path.strip_prefix(cwd.as_str()) {
                stripped.trim_start_matches('/').to_string()
            } else {
                path.clone()
            }
        } else {
            path.clone()
        };

        let mut cursors = Vec::new();
        let mut sels = Vec::new();
        if !selections.is_empty() {
            let mut seen: Vec<(i64, i64)> = selections.clone();
            seen.sort();
            seen.dedup();

            // Collect all offsets in sorted order for a single forward scan
            let all_offsets: Vec<usize> = seen
                .iter()
                .flat_map(|&(s, e)| {
                    if s == e {
                        vec![s as usize]
                    } else {
                        vec![s as usize, e as usize]
                    }
                })
                .collect();

            if let Some(positions) = byte_offsets_to_positions(path, &all_offsets) {
                let mut pos_iter = positions.into_iter();
                for &(start, end) in &seen {
                    let start_pos = pos_iter.next().unwrap();
                    if start == end {
                        cursors.push(start_pos);
                    } else {
                        let end_pos = pos_iter.next().unwrap();
                        sels.push(Selection {
                            start: start_pos,
                            end: end_pos,
                        });
                    }
                }
            }
        }

        files.push(FileEntry {
            path: rel_path,
            cursors,
            selections: sels,
        });
    }

    EditorState { files }
}

/// Find the Zed DB, query editors, and build state for the given cwd.
/// Returns `None` if no DB is found or no files match.
pub fn get_editor_state(cwd: Option<&Path>) -> anyhow::Result<Option<EditorState>> {
    let db_path = match db::find_zed_db() {
        Some(p) => p,
        None => return Ok(None),
    };
    let raw_editors = db::query_editors(&db_path)?;
    let state = build_editor_state(raw_editors, cwd);
    if state.files.is_empty() {
        return Ok(None);
    }
    Ok(Some(state))
}
