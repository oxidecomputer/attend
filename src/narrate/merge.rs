//! Chronological merge of transcription, editor snapshots, and file diffs.
//!
//! Sorts all captured events by wall-clock time, compresses cursor-only
//! snapshot runs, and merges adjacent non-speech events. The actual
//! markdown rendering lives in [`super::render`].

use serde::{Deserialize, Serialize};

use crate::state::FileEntry;

/// A timestamped event from one of the three capture streams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// A transcribed word or group of words.
    Words {
        /// Seconds from recording start.
        offset_secs: f64,
        /// The transcribed text.
        text: String,
    },
    /// An editor state snapshot captured when selections changed.
    EditorSnapshot {
        /// Seconds from recording start.
        offset_secs: f64,
        /// Files with their selections at this point (retained for debugging/archive).
        #[allow(dead_code)]
        files: Vec<FileEntry>,
        /// Pre-rendered view content (from `render_json`).
        rendered: Vec<RenderedFile>,
    },
    /// A file diff captured when file content changed.
    FileDiff {
        /// Seconds from recording start.
        offset_secs: f64,
        /// Absolute path of the changed file.
        path: String,
        /// File content before the change.
        old: String,
        /// File content after the change.
        new: String,
    },
}

/// Pre-rendered file view for an editor snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedFile {
    /// Absolute path of the file.
    pub path: String,
    /// Rendered content with selection markers.
    pub content: String,
    /// First visible line number.
    pub first_line: u32,
}

impl Event {
    /// Sort key: seconds from recording start.
    pub fn offset_secs(&self) -> f64 {
        match self {
            Event::Words { offset_secs, .. }
            | Event::EditorSnapshot { offset_secs, .. }
            | Event::FileDiff { offset_secs, .. } => *offset_secs,
        }
    }
}

/// Produce a unified diff between two strings using the `similar` crate.
pub fn unified_diff(old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.as_str().unwrap_or(""));
        if !change.as_str().unwrap_or("").ends_with('\n') {
            out.push('\n');
        }
    }

    out
}

/// Whether an editor snapshot contains only cursor positions (no real
/// selections).  Cursor-only snapshots from navigation are compressible;
/// snapshots with highlights are always kept because the user may be
/// pointing at multiple things to talk about.
fn is_cursor_only(event: &Event) -> bool {
    let Event::EditorSnapshot { files, .. } = event else {
        return false;
    };
    files
        .iter()
        .all(|f| f.selections.iter().all(|s| s.is_cursor_like()))
}

/// Collapse consecutive cursor-only `EditorSnapshot` runs that have no
/// `Words` between them, keeping only the last snapshot in each run.
/// Snapshots that contain real selections (highlights) are never removed.
fn compress_snapshots(events: &mut Vec<Event>) {
    let mut i = 0;
    while i < events.len() {
        if !is_cursor_only(&events[i]) {
            i += 1;
            continue;
        }
        // Find the end of this cursor-only snapshot run (consecutive
        // snapshots / diffs with no Words in between).
        let mut last_cursor_only = i;
        let mut j = i + 1;
        while j < events.len() && !matches!(events[j], Event::Words { .. }) {
            if is_cursor_only(&events[j]) {
                last_cursor_only = j;
            }
            j += 1;
        }
        // Remove all cursor-only snapshots in the run except the last one.
        let mut k = i;
        while k < j {
            if is_cursor_only(&events[k]) && k != last_cursor_only {
                events.remove(k);
                if last_cursor_only > k {
                    last_cursor_only -= 1;
                }
                j -= 1;
            } else {
                k += 1;
            }
        }
        i = j;
    }
}

/// Merge adjacent non-Words events that aren't separated by speech.
///
/// After cursor compression, a wordless run may still contain multiple
/// selection snapshots and/or file diffs.  This pass:
/// 1. Combines all `EditorSnapshot`s in a run into one whose `rendered`
///    list is the union of all files (every highlight is kept).
/// 2. Combines `FileDiff`s for the same path into one event carrying the
///    first `old` and last `new`, so the rendered diff shows the net
///    change across the whole run.
fn merge_adjacent(events: &mut Vec<Event>) {
    let mut i = 0;
    while i < events.len() {
        if matches!(events[i], Event::Words { .. }) {
            i += 1;
            continue;
        }

        // Determine the extent of this non-Words run.
        let run_start = i;
        let mut j = i + 1;
        while j < events.len() && !matches!(events[j], Event::Words { .. }) {
            j += 1;
        }
        let run_end = j; // exclusive

        if run_end - run_start <= 1 {
            i = run_end;
            continue;
        }

        // --- merge EditorSnapshots (union of all files) ---
        let snapshot_indices: Vec<usize> = (run_start..run_end)
            .filter(|&k| matches!(events[k], Event::EditorSnapshot { .. }))
            .collect();

        if snapshot_indices.len() > 1 {
            let mut merged_files = Vec::new();
            let mut merged_rendered = Vec::new();
            let mut last_offset = 0.0_f64;

            for &idx in &snapshot_indices {
                if let Event::EditorSnapshot {
                    offset_secs,
                    files,
                    rendered,
                } = &events[idx]
                {
                    last_offset = *offset_secs;
                    for f in files {
                        if !merged_files.contains(f) {
                            merged_files.push(f.clone());
                        }
                    }
                    for r in rendered {
                        if !merged_rendered.iter().any(|prev: &RenderedFile| {
                            prev.path == r.path
                                && prev.first_line == r.first_line
                                && prev.content == r.content
                        }) {
                            merged_rendered.push(r.clone());
                        }
                    }
                }
            }

            let last_snap = *snapshot_indices.last().unwrap();
            for &idx in &snapshot_indices {
                if idx != last_snap {
                    events[idx] = Event::EditorSnapshot {
                        offset_secs: 0.0,
                        files: Vec::new(),
                        rendered: Vec::new(),
                    };
                }
            }
            events[last_snap] = Event::EditorSnapshot {
                offset_secs: last_offset,
                files: merged_files,
                rendered: merged_rendered,
            };
        }

        // --- merge FileDiffs (net diff per path) ---
        let diff_indices: Vec<usize> = (run_start..run_end)
            .filter(|&k| matches!(events[k], Event::FileDiff { .. }))
            .collect();

        if diff_indices.len() > 1 {
            // path → (last offset, first old, last new, first index)
            let mut by_path: Vec<(String, f64, String, String, usize)> = Vec::new();

            for &idx in &diff_indices {
                if let Event::FileDiff {
                    offset_secs,
                    path,
                    old,
                    new,
                } = &events[idx]
                {
                    if let Some(entry) = by_path.iter_mut().find(|(p, ..)| p == path) {
                        // Keep the first old, update to the latest new.
                        entry.1 = *offset_secs;
                        entry.3 = new.clone();
                    } else {
                        by_path.push((path.clone(), *offset_secs, old.clone(), new.clone(), idx));
                    }
                }
            }

            for &idx in &diff_indices {
                events[idx] = Event::FileDiff {
                    offset_secs: 0.0,
                    path: String::new(),
                    old: String::new(),
                    new: String::new(),
                };
            }
            for (path, offset_secs, old, new, first_idx) in by_path {
                events[first_idx] = Event::FileDiff {
                    offset_secs,
                    path,
                    old,
                    new,
                };
            }
        }

        // Remove sentinels.
        events.retain(|e| match e {
            Event::EditorSnapshot { rendered, .. } => !rendered.is_empty(),
            Event::FileDiff { path, .. } => !path.is_empty(),
            _ => true,
        });

        i = run_start;
        while i < events.len() && !matches!(events[i], Event::Words { .. }) {
            i += 1;
        }
    }
}

/// Sort events chronologically, compress cursor-only snapshot runs, and
/// merge adjacent non-Words events.
///
/// This is the first phase of `format_markdown` — it mutates `events` in
/// place and is path-format-agnostic (works with both absolute and relative
/// paths).
pub fn compress_and_merge(events: &mut Vec<Event>) {
    events.sort_by(|a, b| {
        a.offset_secs()
            .partial_cmp(&b.offset_secs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    compress_snapshots(events);
    merge_adjacent(events);
}

#[cfg(test)]
mod tests;
