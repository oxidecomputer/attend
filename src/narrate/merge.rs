//! Chronological merge of transcription, editor snapshots, and file diffs.
//!
//! # Event stream format
//!
//! Three capture streams produce [`Event`]s during a narration session:
//!
//! - **Words**: transcribed speech segments from the Whisper model, each
//!   carrying the text and a wall-clock offset. These are the "speech
//!   boundaries" that separate non-speech runs.
//!
//! - **EditorSnapshot**: captured whenever the user's cursor or selection
//!   changes. Contains both the raw `FileEntry` list (for archiving) and
//!   a list of [`CapturedRegion`]s with raw file content and selection
//!   positions. Marker annotation is deferred to render time. A snapshot
//!   is "cursor-only" when every selection is a zero-width cursor; it is a
//!   "selection snapshot" when any selection spans a range (a highlight the
//!   user is pointing at).
//!
//! - **FileDiff**: captured when a watched file's content changes on disk.
//!   Carries the full `old` and `new` content so the merge pipeline can
//!   compute net changes across multiple edits.
//!
//! All events share an `timestamp` field: seconds from recording start.
//!
//! # Merge pipeline
//!
//! [`compress_and_merge`] processes the raw event stream in three steps:
//!
//! 1. **Chronological sort** by `timestamp`.
//!
//! 2. **Single-pass run processing**: the sorted list is split into
//!    alternating `Words` events and non-speech "runs" (maximal sequences
//!    of `EditorSnapshot` / `FileDiff` with no `Words` between them). Each
//!    run is processed through three composable transformations:
//!
//!    - **Cursor compression** ([`collapse_cursor_only`]): removes all
//!      cursor-only snapshots except the last in each run. Rapid navigation
//!      (opening files, scrolling) generates many cursor events; only the
//!      final position before the next utterance matters. Selection snapshots
//!      (highlights) are never removed because they represent deliberate
//!      pointing at code.
//!
//!    - **Snapshot union** ([`union_snapshots`]): folds the surviving
//!      snapshots into a single snapshot whose region list is the
//!      deduplicated union of every region. This ensures every file the user
//!      looked at between two utterances appears in one cohesive code block.
//!
//!    - **Diff net-change** ([`net_change_diffs`]): groups diffs by file
//!      path and keeps only the first `old` and last `new`. If a file
//!      changed A→B→C between two utterances, the rendered diff shows A→C
//!      (the net change). If the file was changed and then reverted (A→B→A),
//!      the net diff is empty and the event is dropped at render time.
//!
//! 3. **Trailing cursor drop**: if speech is present, a final cursor-only
//!    snapshot is removed because the stop hook provides more up-to-date
//!    editor context. Code-only narrations (no speech at all) keep
//!    everything.
//!
//! The actual markdown rendering lives in [`super::render`].

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::state::FileEntry;
pub use crate::view::CapturedRegion;

/// The kind of event that was redacted (filtered due to project scope).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RedactedKind {
    EditorSnapshot,
    FileDiff,
    ShellCommand,
}

/// A timestamped event from one of the capture streams.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    /// A transcribed word or group of words.
    Words {
        /// UTC wall-clock time when the word was spoken.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// The transcribed text.
        text: String,
    },
    /// An editor state snapshot captured when selections changed.
    EditorSnapshot {
        /// UTC wall-clock time of capture.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// Files with their selections at this point (retained for debugging/archive).
        #[allow(dead_code)]
        files: Vec<FileEntry>,
        /// Captured regions with raw content and selection positions.
        regions: Vec<CapturedRegion>,
    },
    /// A file diff captured when file content changed.
    FileDiff {
        /// UTC wall-clock time of capture.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// Absolute path of the changed file.
        path: String,
        /// File content before the change.
        old: String,
        /// File content after the change.
        new: String,
    },
    /// Text selected in an external application (via platform accessibility API).
    ExternalSelection {
        /// UTC wall-clock time of capture.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// Application name (e.g. "Firefox", "iTerm2", "Safari").
        app: String,
        /// Window title (e.g. page title, terminal tab name).
        window_title: String,
        /// The selected text.
        text: String,
    },
    /// Text selected in a browser, with rich page context.
    /// Delivered via a browser extension's native messaging bridge.
    BrowserSelection {
        /// UTC wall-clock time of capture.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// Page URL.
        url: String,
        /// Page title.
        title: String,
        /// The selected content, converted from HTML to markdown by the bridge.
        text: String,
    },
    /// A command executed in the user's shell.
    /// Delivered via the `attend shell-hook` CLI subcommand.
    ShellCommand {
        /// UTC wall-clock time when the command started.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// The shell (e.g. "fish", "zsh").
        shell: String,
        /// The command as typed by the user.
        command: String,
        /// Working directory when the command was executed.
        cwd: String,
        /// Exit status (None for preexec-only, before the command completes).
        exit_status: Option<i32>,
        /// Wall-clock duration in seconds (None for preexec-only).
        duration_secs: Option<f64>,
    },
    /// A placeholder for events filtered out by project-scope checks.
    ///
    /// Created during the receive phase when events fall outside the project
    /// directory and configured `include_dirs`. Never serialized to disk.
    Redacted {
        /// UTC wall-clock time of the original event.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// What kind of event was redacted.
        kind: RedactedKind,
        /// Identifiers for deduplication during collapse. For EditorSnapshot
        /// and FileDiff these are file paths; for ShellCommand, command text.
        keys: Vec<String>,
    },
}

impl Event {
    /// Sort key: UTC timestamp.
    pub fn timestamp(&self) -> chrono::DateTime<chrono::Utc> {
        match self {
            Event::Words { timestamp, .. }
            | Event::EditorSnapshot { timestamp, .. }
            | Event::FileDiff { timestamp, .. }
            | Event::ExternalSelection { timestamp, .. }
            | Event::BrowserSelection { timestamp, .. }
            | Event::ShellCommand { timestamp, .. }
            | Event::Redacted { timestamp, .. } => *timestamp,
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
/// **Input**: a non-Words run (sorted by timestamp, no `Words` events).
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

/// Fold all `EditorSnapshot`s into a single snapshot whose region list
/// is the deduplicated union of every region. Uses the last snapshot's
/// offset as the merged offset (chronologically latest).
///
/// **Input**: snapshots extracted from a run, in chronological order.
/// **Output**: a single `(offset, files, regions)` tuple, or `None` if
/// the input was empty.
///
/// **Invariant**: every unique `CapturedRegion` from the input appears in
/// the output exactly once (dedup via `PartialEq`).
fn union_snapshots(
    snapshots: Vec<(
        chrono::DateTime<chrono::Utc>,
        Vec<FileEntry>,
        Vec<CapturedRegion>,
    )>,
) -> Option<(
    chrono::DateTime<chrono::Utc>,
    Vec<FileEntry>,
    Vec<CapturedRegion>,
)> {
    if snapshots.is_empty() {
        return None;
    }

    let mut merged_files = Vec::new();
    let mut seen_files = HashSet::new();
    let mut merged_regions = Vec::new();
    let mut seen_regions = HashSet::new();
    let mut last_ts = snapshots[0].0;

    for (ts, files, regions) in snapshots {
        last_ts = ts;
        for f in files {
            if seen_files.insert(f.clone()) {
                merged_files.push(f);
            }
        }
        for r in regions {
            if seen_regions.insert(r.clone()) {
                merged_regions.push(r);
            }
        }
    }

    Some((last_ts, merged_files, merged_regions))
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
    diffs: Vec<(chrono::DateTime<chrono::Utc>, String, String, String)>,
) -> Vec<(chrono::DateTime<chrono::Utc>, String, String, String)> {
    let mut by_path: Vec<(chrono::DateTime<chrono::Utc>, String, String, String)> = Vec::new();

    for (ts, path, old, new) in diffs {
        if let Some(entry) = by_path.iter_mut().find(|(_, p, ..)| p == &path) {
            entry.0 = ts; // latest timestamp
            entry.3 = new; // latest new
        } else {
            by_path.push((ts, path, old, new));
        }
    }

    by_path
}

/// Collapse duplicate external and browser selection events within a run.
///
/// **Input**: external/browser selection events from a run, in chronological order.
/// **Output**: deduplicated events with progressive selections forward-merged.
///
/// **Forward-merge rule** (ExternalSelection, same app + window_title):
/// When an earlier selection's text is a substring of a later one, the earlier
/// is dropped (the user was progressively selecting more text). When the later
/// selection is *narrower* (a substring of an earlier one), it starts a new
/// chain — both survive, and the new selection becomes the merge target for
/// future extensions.
///
/// **BrowserSelection**: deduplicated by (url, text) — keep latest per pair.
///
/// **Cross-type**: when a BrowserSelection and ExternalSelection have matching
/// text within 500ms, the ExternalSelection is dropped.
fn collapse_ext_selections(selections: Vec<Event>) -> Vec<Event> {
    let mut result: Vec<Event> = Vec::new();

    for event in selections {
        match &event {
            Event::ExternalSelection {
                app,
                window_title,
                text,
                ..
            } => {
                // Forward-merge: check if an earlier selection from the same
                // source is a substring of this one (progressive selection).
                let merged = result.iter().rposition(|e| {
                    matches!(
                        e,
                        Event::ExternalSelection {
                            app: a,
                            window_title: wt,
                            text: t,
                            ..
                        } if a == app && wt == window_title && text.contains(t.as_str())
                    )
                });
                if let Some(idx) = merged {
                    // Replace the earlier, narrower selection with this wider one.
                    result[idx] = event;
                } else {
                    result.push(event);
                }
            }
            Event::BrowserSelection { url, text, .. } => {
                // Replace a previous entry with the same url + text.
                if let Some(existing) = result.iter_mut().find(|e| {
                    matches!(e, Event::BrowserSelection { url: u, text: t, .. } if u == url && t == text)
                }) {
                    *existing = event;
                } else {
                    result.push(event);
                }
            }
            Event::ShellCommand {
                command,
                exit_status,
                ..
            } => {
                // Preexec/postexec dedup: if earlier preexec(s) (exit_status=None)
                // for the same command text exist, remove them all and push the
                // postexec (which has richer data: exit status + duration).
                if exit_status.is_some() {
                    result.retain(|e| {
                        !matches!(
                            e,
                            Event::ShellCommand {
                                command: c,
                                exit_status: None,
                                ..
                            } if c == command
                        )
                    });
                    result.push(event);
                } else {
                    result.push(event);
                }
            }
            _ => {
                result.push(event);
            }
        }
    }

    // Cross-type dedup: when BrowserSelection and ExternalSelection have the
    // same text within 500ms, drop the ExternalSelection (browser is richer).
    dedup_browser_vs_external(&mut result);

    result
}

/// Drop `ExternalSelection` events that are superseded by a nearby
/// `BrowserSelection` with matching text (trimmed).
///
/// "Nearby" means within 500ms (0.5 seconds). The browser extension provides
/// richer context (URL, HTML→markdown) than the accessibility API, so the browser
/// event wins.
fn dedup_browser_vs_external(events: &mut Vec<Event>) {
    let dedup_window = chrono::Duration::milliseconds(500);

    // Collect browser selection timestamps and texts for matching.
    let browser_entries: Vec<(chrono::DateTime<chrono::Utc>, String)> = events
        .iter()
        .filter_map(|e| match e {
            Event::BrowserSelection {
                timestamp, text, ..
            } => Some((*timestamp, text.trim().to_string())),
            _ => None,
        })
        .collect();

    if browser_entries.is_empty() {
        return;
    }

    events.retain(|e| {
        let Event::ExternalSelection {
            timestamp, text, ..
        } = e
        else {
            return true;
        };
        let trimmed = text.trim();
        // Drop if any BrowserSelection has matching text within the window.
        !browser_entries.iter().any(|(bs_ts, bs_text)| {
            bs_text == trimmed && (*timestamp - *bs_ts).abs().le(&dedup_window)
        })
    });
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
/// - Output is sorted by timestamp.
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
    let mut ext_selections = Vec::new();

    for event in run {
        match event {
            Event::EditorSnapshot {
                timestamp,
                files,
                regions,
            } => {
                snapshots.push((timestamp, files, regions));
            }
            Event::FileDiff {
                timestamp,
                path,
                old,
                new,
            } => {
                diffs.push((timestamp, path, old, new));
            }
            Event::ExternalSelection { .. } | Event::BrowserSelection { .. } => {
                ext_selections.push(event);
            }
            Event::ShellCommand { .. } => {
                ext_selections.push(event);
            }
            // Redacted events are created after merge; pass through if present.
            Event::Redacted { .. } => {
                ext_selections.push(event);
            }
            Event::Words { .. } => unreachable!("run should not contain Words"),
        }
    }

    let merged_snap = union_snapshots(snapshots);
    let merged_diffs = net_change_diffs(diffs);
    let merged_ext = collapse_ext_selections(ext_selections);

    // Reassemble in chronological order.
    let mut result = Vec::with_capacity(1 + merged_diffs.len() + merged_ext.len());

    if let Some((timestamp, files, regions)) = merged_snap
        && !regions.is_empty()
    {
        result.push(Event::EditorSnapshot {
            timestamp,
            files,
            regions,
        });
    }

    for (timestamp, path, old, new) in merged_diffs {
        result.push(Event::FileDiff {
            timestamp,
            path,
            old,
            new,
        });
    }

    result.extend(merged_ext);

    result.sort_by_key(|a| a.timestamp());

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
    events.sort_by_key(|a| a.timestamp());

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
