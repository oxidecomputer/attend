//! Chronological merge of transcription, editor snapshots, and file diffs.
//!
//! The merge pipeline sorts all captured events by wall-clock time, then
//! makes a single pass over the event list, processing each non-speech run
//! through three composable transformations:
//!
//! 1. **Cursor compression** ([`collapse_cursor_only`]): Within a run,
//!    removes all cursor-only snapshots except the last one. Navigation
//!    chatter is reduced while keeping the final cursor position.
//!
//! 2. **Snapshot union** ([`union_snapshots`]): Folds all surviving snapshots
//!    in a run into a single snapshot whose rendered file list is the union
//!    of every file the user looked at.
//!
//! 3. **Diff net-change** ([`net_change_diffs`]): Groups diffs by file path
//!    and keeps only the first `old` and last `new` for each path, so the
//!    rendered diff shows the net change across the entire run.
//!
//! The actual markdown rendering lives in [`super::render`].

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

// ── Composable run transformations ──────────────────────────────────────────

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

/// Remove all cursor-only snapshots from a run except the last one.
///
/// **Input**: a non-Words run (sorted by offset_secs, no `Words` events).
/// **Output**: the same run with redundant cursor-only snapshots removed.
///
/// **Invariant**: selection (highlight) snapshots are never removed.
/// **Invariant**: at most one cursor-only snapshot survives per run.
fn collapse_cursor_only(run: &mut Vec<Event>) {
    let Some(keep) = run.iter().rposition(is_cursor_only) else {
        return;
    };
    let mut idx = 0;
    run.retain(|e| {
        let i = idx;
        idx += 1;
        !is_cursor_only(e) || i == keep
    });
}

/// Fold all `EditorSnapshot`s into a single snapshot whose rendered file
/// list is the deduplicated union of every file. Uses the last snapshot's
/// offset as the merged offset (chronologically latest).
///
/// **Input**: snapshots extracted from a run, in chronological order.
/// **Output**: a single `(offset, files, rendered)` tuple, or `None` if
/// the input was empty.
///
/// **Invariant**: every unique `(path, first_line, content)` from the
/// input appears in the output exactly once.
fn union_snapshots(
    snapshots: Vec<(f64, Vec<FileEntry>, Vec<RenderedFile>)>,
) -> Option<(f64, Vec<FileEntry>, Vec<RenderedFile>)> {
    if snapshots.is_empty() {
        return None;
    }

    let mut merged_files = Vec::new();
    let mut merged_rendered = Vec::new();
    let mut last_offset = 0.0_f64;

    for (offset, files, rendered) in snapshots {
        last_offset = offset;
        for f in files {
            if !merged_files.contains(&f) {
                merged_files.push(f);
            }
        }
        for r in rendered {
            if !merged_rendered.iter().any(|prev: &RenderedFile| {
                prev.path == r.path && prev.first_line == r.first_line && prev.content == r.content
            }) {
                merged_rendered.push(r);
            }
        }
    }

    Some((last_offset, merged_files, merged_rendered))
}

/// Collapse same-path diffs into net-change events (first `old`, last `new`).
/// Returns one diff per unique path, in first-seen order.
///
/// **Input**: diffs extracted from a run, in chronological order.
/// **Output**: one `(offset, path, old, new)` per unique path. The offset
/// is the latest for that path.
///
/// **Invariant**: for each path, `old` is from the earliest diff and `new`
/// is from the latest diff in the input.
fn net_change_diffs(
    diffs: Vec<(f64, String, String, String)>,
) -> Vec<(f64, String, String, String)> {
    let mut by_path: Vec<(f64, String, String, String)> = Vec::new();

    for (offset, path, old, new) in diffs {
        if let Some(entry) = by_path.iter_mut().find(|(_, p, ..)| p == &path) {
            entry.0 = offset; // latest offset
            entry.3 = new; // latest new
        } else {
            by_path.push((offset, path, old, new));
        }
    }

    by_path
}

// ── Run processing ──────────────────────────────────────────────────────────

/// Process a single non-Words run through the three composable
/// transformations: cursor compression, snapshot union, diff net-change.
///
/// **Input**: sorted non-Words events from a single run (between `Words`
/// boundaries).
/// **Output**: compressed events in chronological order.
///
/// **Invariants** (post-conditions):
/// - At most one `EditorSnapshot` in the output.
/// - At most one `FileDiff` per path in the output.
/// - No cursor-only snapshots survive unless they are the sole cursor-only
///   in the run (the last one).
/// - Output is sorted by offset_secs.
fn process_run(mut run: Vec<Event>) -> Vec<Event> {
    if run.len() <= 1 {
        return run;
    }

    // Phase 1: collapse cursor-only snapshots.
    collapse_cursor_only(&mut run);

    if run.len() <= 1 {
        return run;
    }

    // Phase 2 & 3: partition, then union snapshots and net-change diffs.
    let mut snapshots = Vec::new();
    let mut diffs = Vec::new();

    for event in run {
        match event {
            Event::EditorSnapshot {
                offset_secs,
                files,
                rendered,
            } => {
                snapshots.push((offset_secs, files, rendered));
            }
            Event::FileDiff {
                offset_secs,
                path,
                old,
                new,
            } => {
                diffs.push((offset_secs, path, old, new));
            }
            Event::Words { .. } => unreachable!("run should not contain Words"),
        }
    }

    let merged_snap = union_snapshots(snapshots);
    let merged_diffs = net_change_diffs(diffs);

    // Reassemble in chronological order.
    let mut result = Vec::with_capacity(1 + merged_diffs.len());

    if let Some((offset_secs, files, rendered)) = merged_snap
        && !rendered.is_empty()
    {
        result.push(Event::EditorSnapshot {
            offset_secs,
            files,
            rendered,
        });
    }

    for (offset_secs, path, old, new) in merged_diffs {
        result.push(Event::FileDiff {
            offset_secs,
            path,
            old,
            new,
        });
    }

    result.sort_by(|a, b| {
        a.offset_secs()
            .partial_cmp(&b.offset_secs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    result
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Sort events chronologically and process all non-speech runs in a single
/// pass: compress cursor-only snapshots, union adjacent snapshots, and
/// merge same-path diffs into net-change events.
///
/// This is the first phase of `format_markdown` — it mutates `events` in
/// place and is path-format-agnostic (works with both absolute and relative
/// paths).
pub fn compress_and_merge(events: &mut Vec<Event>) {
    // Step 1: chronological sort.
    events.sort_by(|a, b| {
        a.offset_secs()
            .partial_cmp(&b.offset_secs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Step 2: single pass — split on Words boundaries, process each run.
    let mut output = Vec::with_capacity(events.len());
    let mut run = Vec::new();

    for event in events.drain(..) {
        if matches!(event, Event::Words { .. }) {
            if !run.is_empty() {
                output.extend(process_run(std::mem::take(&mut run)));
            }
            output.push(event);
        } else {
            run.push(event);
        }
    }
    if !run.is_empty() {
        output.extend(process_run(run));
    }

    // Step 3: drop trailing cursor-only snapshot when speech is present.
    // The stop hook already provides the latest editor context, which is
    // more up-to-date. For code-only narrations (no speech), keep everything.
    let has_words = output.iter().any(|e| matches!(e, Event::Words { .. }));
    if has_words && output.last().is_some_and(is_cursor_only) {
        output.pop();
    }

    *events = output;
}

#[cfg(test)]
mod tests;
