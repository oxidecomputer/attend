//! Chronological merge and compression of narration events.
//!
//! # Event sources
//!
//! Seven event types arrive from six capture sources during a narration
//! session:
//!
//! - **Words**: transcribed speech segments from the Whisper model. These
//!   are the "speech boundaries" that separate non-speech runs.
//!
//! - **EditorSnapshot**: polled from the editor whenever the cursor or
//!   selection changes. Contains a `FileEntry` list (for archiving) and
//!   [`CapturedRegion`]s with raw file content. A snapshot is "cursor-only"
//!   when every selection is a zero-width cursor; it is a "selection
//!   snapshot" when any selection spans a range.
//!
//! - **FileDiff**: captured when a watched file's content changes on disk.
//!   Carries full `old` and `new` content for net-change computation.
//!
//! - **ExternalSelection**: polled from the focused application via
//!   platform accessibility APIs (macOS AX). Carries app name, window
//!   title, and selected text.
//!
//! - **BrowserSelection**: pushed from the browser extension via native
//!   messaging. Carries URL, page title, and HTML-to-markdown content.
//!
//! - **ShellCommand**: pushed from shell hook integration (preexec/postexec).
//!   Carries command text, cwd, exit status, and duration.
//!
//! - **ClipboardSelection**: captured when clipboard content changes during
//!   a session. Text content is stored inline; images are PNG-encoded and
//!   staged to a file. Point-in-time only (no `last_seen`).
//!
//! All events carry a UTC wall-clock `timestamp` and selection-bearing
//! types also carry a `last_seen` timestamp (the last time the selection
//! was observed unchanged by the capture layer).
//!
//! # Merge pipeline
//!
//! [`compress_and_merge`] processes the raw event stream in four steps:
//!
//! 1. **Chronological sort** by `timestamp`.
//!
//! 2. **Global progressive selection subsumption**
//!    ([`subsume_progressive_selections`]): drops earlier, narrower
//!    selections when a later, wider selection from the same source
//!    arrives within 2s of the earlier event's `last_seen`. The gap is
//!    measured from `last_seen` (not `timestamp`), giving temporal
//!    continuity when a selection is held across poll ticks. Runs before
//!    run-splitting so intermediate events serve as temporal bridges for
//!    chain subsumption (A ⊂ B ⊂ C each within tolerance).
//!
//! 3. **Single-pass run processing**: the sorted list is split into
//!    alternating `Words` events and non-speech "runs" (maximal sequences
//!    with no `Words` between them). Each run is processed through
//!    composable transformations:
//!
//!    - **Cursor compression** ([`collapse_cursor_only`]): removes all
//!      cursor-only snapshots except the last in each run.
//!
//!    - **Snapshot union** ([`union_snapshots`]): folds surviving snapshots
//!      into a single snapshot with the deduplicated union of all regions.
//!
//!    - **Diff net-change** ([`net_change_diffs`]): groups diffs by path,
//!      keeps first `old` and last `new`. Reverted changes (A→B→A) produce
//!      an empty diff dropped at render time.
//!
//!    - **Selection/command collapse** ([`collapse_ext_selections`]):
//!      forward-merges progressive ExternalSelections, deduplicates
//!      BrowserSelections by (url, text), and merges preexec/postexec
//!      ShellCommands.
//!
//!    - **Cross-type dedup** ([`dedup_browser_vs_external`]): drops
//!      ExternalSelections superseded by a nearby BrowserSelection with
//!      matching text.
//!
//! 4. **Trailing cursor drop**: if speech is present, a final cursor-only
//!    snapshot is removed (the stop hook provides fresher editor context).
//!    Code-only narrations keep everything.
//!
//! The actual markdown rendering lives in [`super::render`].

use std::collections::HashSet;
use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::state::FileEntry;
pub use crate::view::CapturedRegion;

/// Default value for `last_seen` when deserializing old archives that
/// lack the field. Downstream code treats epoch as "no data" and falls
/// back to `timestamp`.
fn epoch() -> DateTime<Utc> {
    DateTime::UNIX_EPOCH
}

/// Window (ms) for cross-type dedup: BrowserSelection vs ExternalSelection.
const CROSS_TYPE_DEDUP_WINDOW_MS: i64 = 500;

/// Window (ms) for cross-run progressive selection subsumption.
///
/// When an earlier selection's text is a substring of a later selection from
/// the same source within this window, the earlier event is dropped. The poll
/// interval is 200ms and typical speech between captures is 500ms–1.5s, so
/// 2 seconds is generous without merging unrelated selections.
const SUBSUME_WINDOW_MS: i64 = 2000;

/// Content captured from the system clipboard.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClipboardContent {
    /// Plain text copied to the clipboard.
    Text { text: String },
    /// Image copied to the clipboard, staged as a PNG file.
    Image { path: String },
}

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
        timestamp: DateTime<Utc>,
        /// The transcribed text.
        text: String,
    },
    /// An editor state snapshot captured when selections changed.
    EditorSnapshot {
        /// UTC wall-clock time of capture.
        timestamp: DateTime<Utc>,
        /// UTC wall-clock time when this selection was last observed unchanged.
        /// Set equal to `timestamp` on creation; extended by the capture layer
        /// when the selection persists across poll ticks.
        #[serde(default = "epoch")]
        last_seen: DateTime<Utc>,
        /// Files with their selections at this point (retained for debugging/archive).
        #[allow(dead_code)]
        files: Vec<FileEntry>,
        /// Captured regions with raw content and selection positions.
        regions: Vec<CapturedRegion>,
    },
    /// A file diff captured when file content changed.
    FileDiff {
        /// UTC wall-clock time of capture.
        timestamp: DateTime<Utc>,
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
        timestamp: DateTime<Utc>,
        /// UTC wall-clock time when this selection was last observed unchanged.
        #[serde(default = "epoch")]
        last_seen: DateTime<Utc>,
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
        timestamp: DateTime<Utc>,
        /// UTC wall-clock time when this selection was last observed unchanged.
        #[serde(default = "epoch")]
        last_seen: DateTime<Utc>,
        /// Page URL.
        url: String,
        /// Page title.
        title: String,
        /// The selected content, converted from HTML to markdown by the bridge.
        text: String,
        /// Plain-text rendering of the selection (`selection.toString()` from
        /// the browser). Used for dedup against clipboard and external
        /// selections, since the `text` field contains markdown.
        #[serde(default)]
        plain_text: String,
    },
    /// A command executed in the user's shell.
    /// Delivered via the `attend shell-hook` CLI subcommand.
    ShellCommand {
        /// UTC wall-clock time when the command started.
        timestamp: DateTime<Utc>,
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
    /// Content captured from the system clipboard.
    ///
    /// Emitted once per clipboard change during a recording session. Text is
    /// stored inline; images are PNG-encoded and staged to a file. No `last_seen`
    /// field — clipboard captures are point-in-time only.
    ClipboardSelection {
        /// UTC wall-clock time of capture.
        timestamp: DateTime<Utc>,
        /// The clipboard content (text or image).
        content: ClipboardContent,
    },
    /// A placeholder for events filtered out by project-scope checks.
    ///
    /// Created during the receive phase when events fall outside the project
    /// directory and configured `include_dirs`. Never serialized to disk.
    Redacted {
        /// UTC wall-clock time of the original event.
        timestamp: DateTime<Utc>,
        /// What kind of event was redacted.
        kind: RedactedKind,
        /// Identifiers for deduplication during collapse. For EditorSnapshot
        /// and FileDiff these are file paths; for ShellCommand, command text.
        keys: Vec<String>,
    },
}

impl Event {
    /// Sort key: UTC timestamp.
    pub fn timestamp(&self) -> DateTime<Utc> {
        match self {
            Event::Words { timestamp, .. }
            | Event::EditorSnapshot { timestamp, .. }
            | Event::FileDiff { timestamp, .. }
            | Event::ExternalSelection { timestamp, .. }
            | Event::BrowserSelection { timestamp, .. }
            | Event::ShellCommand { timestamp, .. }
            | Event::ClipboardSelection { timestamp, .. }
            | Event::Redacted { timestamp, .. } => *timestamp,
        }
    }

    /// The last time this event's selection was observed unchanged.
    ///
    /// For selection-bearing types (`ExternalSelection`, `BrowserSelection`,
    /// `EditorSnapshot`): returns `last_seen`, falling back to `timestamp`
    /// when the value is epoch (old archives that lack the field).
    ///
    /// For all other types: returns `timestamp`.
    pub fn last_seen(&self) -> DateTime<Utc> {
        match self {
            Event::ExternalSelection {
                timestamp,
                last_seen,
                ..
            }
            | Event::BrowserSelection {
                timestamp,
                last_seen,
                ..
            }
            | Event::EditorSnapshot {
                timestamp,
                last_seen,
                ..
            } => {
                if *last_seen == DateTime::UNIX_EPOCH {
                    *timestamp
                } else {
                    *last_seen
                }
            }
            _ => self.timestamp(),
        }
    }
}

impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Event::Words { timestamp, text } => {
                let ts = timestamp.format("%H:%M:%S");
                let mut chars = text.chars();
                let preview: String = (&mut chars).take(40).collect();
                let truncated = chars.next().is_some();
                write!(
                    f,
                    "Words[{ts}]: \"{preview}{}\"",
                    if truncated { "..." } else { "" }
                )
            }
            Event::EditorSnapshot {
                timestamp, regions, ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                let count = regions.len();
                write!(f, "EditorSnapshot[{ts}]: {count} region(s)")
            }
            Event::FileDiff {
                timestamp, path, ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                write!(f, "FileDiff[{ts}]: {path}")
            }
            Event::ExternalSelection {
                timestamp,
                app,
                window_title,
                ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                write!(f, "ExternalSelection[{ts}]: {app} - {window_title}")
            }
            Event::BrowserSelection {
                timestamp,
                url,
                title,
                ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                write!(f, "BrowserSelection[{ts}]: {title} ({url})")
            }
            Event::ShellCommand {
                timestamp,
                command,
                exit_status,
                ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                let mut chars = command.chars();
                let preview: String = (&mut chars).take(40).collect();
                let truncated = chars.next().is_some();
                match exit_status {
                    Some(code) => write!(
                        f,
                        "ShellCommand[{ts}]: \"{preview}{}\" (exit {code})",
                        if truncated { "..." } else { "" }
                    ),
                    None => write!(
                        f,
                        "ShellCommand[{ts}]: \"{preview}{}\" (running)",
                        if truncated { "..." } else { "" }
                    ),
                }
            }
            Event::ClipboardSelection {
                timestamp, content, ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                match content {
                    ClipboardContent::Text { text } => {
                        let mut chars = text.chars();
                        let preview: String = (&mut chars).take(40).collect();
                        let truncated = chars.next().is_some();
                        write!(
                            f,
                            "ClipboardSelection[{ts}]: text \"{preview}{}\"",
                            if truncated { "..." } else { "" }
                        )
                    }
                    ClipboardContent::Image { path } => {
                        write!(f, "ClipboardSelection[{ts}]: image {path}")
                    }
                }
            }
            Event::Redacted {
                timestamp, kind, ..
            } => {
                let ts = timestamp.format("%H:%M:%S");
                write!(f, "Redacted[{ts}]: {kind:?}")
            }
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

// ── Global passes ───────────────────────────────────────────────────────────
//
// Transformations that operate on the full chronologically sorted event
// list *before* run-splitting.

/// Whether an editor snapshot contains only cursor positions (no real
/// selections). Cursor-only snapshots from navigation are compressible;
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

/// Pre-extracted match key for an event, built once per event in O(n) total.
/// The inner loop reads from `keys[j]` instead of borrowing from `events[j]`,
/// eliminating the borrow conflict that previously required O(n²) cloning.
enum SubsumeKey {
    /// ExternalSelection: (app, window_title, text).
    External(String, String, String),
    /// BrowserSelection: (url, text, plain_text).
    Browser(String, String, String),
    /// EditorSnapshot: Vec<(path, content)> from regions.
    Editor(Vec<(String, String)>),
    /// ClipboardSelection with text content.
    Clipboard(String),
    /// Event types that don't participate in subsumption.
    Inert,
}

/// Drop earlier, narrower selections when a later, wider selection from the
/// same source arrives within [`SUBSUME_WINDOW_MS`].
///
/// The gap is measured from `last_seen` of the earlier event to `timestamp`
/// of the later event, giving temporal continuity when a selection is held
/// across multiple poll ticks. For old data without `last_seen`, the field
/// defaults to `timestamp` (no behavior change).
///
/// Operates on the chronologically sorted event list before run-splitting, so
/// intermediate events (including words) act as temporal bridges for chain
/// subsumption: A ⊂ B ⊂ C each within tolerance even if A→C exceeds it.
///
/// Applies uniformly to `ExternalSelection` (keyed on app + window_title),
/// `BrowserSelection` (keyed on url), and `EditorSnapshot` (keyed on region
/// paths, where every region must be covered for subsumption).
fn subsume_progressive_selections(events: &mut Vec<Event>) {
    let tolerance = chrono::Duration::milliseconds(SUBSUME_WINDOW_MS);
    let len = events.len();
    let mut remove = vec![false; len];

    // O(n) key extraction: one clone per event field, total.
    let keys: Vec<SubsumeKey> = events
        .iter()
        .map(|e| match e {
            Event::ExternalSelection {
                app,
                window_title,
                text,
                ..
            } => SubsumeKey::External(app.clone(), window_title.clone(), text.clone()),
            Event::BrowserSelection {
                url,
                text,
                plain_text,
                ..
            } => SubsumeKey::Browser(url.clone(), text.clone(), plain_text.clone()),
            Event::EditorSnapshot { regions, .. } => SubsumeKey::Editor(
                regions
                    .iter()
                    .map(|r| (r.path.clone(), r.content.clone()))
                    .collect(),
            ),
            Event::ClipboardSelection {
                content: ClipboardContent::Text { text },
                ..
            } => SubsumeKey::Clipboard(text.clone()),
            _ => SubsumeKey::Inert,
        })
        .collect();

    for i in 0..len {
        if remove[i] {
            continue;
        }
        let ls_i = events[i].last_seen();

        match &keys[i] {
            SubsumeKey::External(app, wt, text) => {
                for j in (i + 1)..len {
                    if (events[j].timestamp() - ls_i) > tolerance {
                        break;
                    }
                    if let SubsumeKey::External(a, w, t) = &keys[j]
                        && a == app
                        && w == wt
                        && t.contains(text.as_str())
                    {
                        remove[i] = true;
                        break;
                    }
                }
            }
            SubsumeKey::Browser(url, text, _) => {
                for j in (i + 1)..len {
                    if (events[j].timestamp() - ls_i) > tolerance {
                        break;
                    }
                    if let SubsumeKey::Browser(u, t, _) = &keys[j]
                        && u == url
                        && t.contains(text.as_str())
                    {
                        remove[i] = true;
                        break;
                    }
                }
            }
            SubsumeKey::Editor(regions_i) => {
                if regions_i.is_empty() {
                    continue;
                }
                for j in (i + 1)..len {
                    if (events[j].timestamp() - ls_i) > tolerance {
                        break;
                    }
                    if let SubsumeKey::Editor(regions_j) = &keys[j] {
                        // Every region in i must have a matching region in j
                        // (same path, content contains).
                        let covered = regions_i.iter().all(|(path_i, content_i)| {
                            regions_j
                                .iter()
                                .any(|(pj, cj)| pj == path_i && cj.contains(content_i.as_str()))
                        });
                        if covered {
                            remove[i] = true;
                            break;
                        }
                    }
                }
            }
            SubsumeKey::Clipboard(text) => {
                for j in (i + 1)..len {
                    if (events[j].timestamp() - ls_i) > tolerance {
                        break;
                    }
                    // Clipboard can be subsumed by ExternalSelection.
                    if let SubsumeKey::External(_, _, t) = &keys[j]
                        && t.contains(text.as_str())
                    {
                        remove[i] = true;
                        break;
                    }
                    // Clipboard can be subsumed by BrowserSelection (via plain_text).
                    if let SubsumeKey::Browser(_, _, plain_text) = &keys[j]
                        && plain_text.contains(text.as_str())
                    {
                        remove[i] = true;
                        break;
                    }
                    // Clipboard can be subsumed by another ClipboardSelection.
                    if let SubsumeKey::Clipboard(t) = &keys[j]
                        && t.contains(text.as_str())
                    {
                        remove[i] = true;
                        break;
                    }
                }
            }
            SubsumeKey::Inert => {}
        }
    }

    let mut idx = 0;
    events.retain(|_| {
        let keep = !remove[idx];
        idx += 1;
        keep
    });
}

// ── Per-run transformations ─────────────────────────────────────────────────
//
// Each function below operates on a single non-Words run (a maximal
// sequence of events between speech boundaries).

/// Extracted fields from an `EditorSnapshot` for union processing.
type SnapshotTuple = (
    DateTime<Utc>,
    DateTime<Utc>,
    Vec<FileEntry>,
    Vec<CapturedRegion>,
);

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
/// timestamps (chronologically latest).
///
/// **Input**: snapshots extracted from a run, in chronological order.
/// **Output**: a single `(timestamp, last_seen, files, regions)` tuple,
/// or `None` if the input was empty.
///
/// **Invariant**: every unique `CapturedRegion` from the input appears in
/// the output exactly once (dedup via `PartialEq`).
fn union_snapshots(snapshots: Vec<SnapshotTuple>) -> Option<SnapshotTuple> {
    if snapshots.is_empty() {
        return None;
    }

    let mut merged_files = Vec::new();
    let mut seen_files = HashSet::new();
    let mut merged_regions = Vec::new();
    let mut seen_regions = HashSet::new();
    let mut last_ts = snapshots[0].0;
    let mut last_ls = snapshots[0].1;

    for (ts, ls, files, regions) in snapshots {
        last_ts = ts;
        last_ls = ls;
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

    Some((last_ts, last_ls, merged_files, merged_regions))
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
    diffs: Vec<(DateTime<Utc>, String, String, String)>,
) -> Vec<(DateTime<Utc>, String, String, String)> {
    let mut by_path: Vec<(DateTime<Utc>, String, String, String)> = Vec::new();

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
                // Preexec/postexec dedup: merge preexec + postexec into one
                // event with the preexec's cwd (the command's actual working
                // directory) and the postexec's exit status + duration.
                //
                // The preexec cwd is correct because it captures where the
                // user was when they typed the command. The postexec cwd may
                // differ for directory-changing commands (e.g. `cd ..`).
                if exit_status.is_some() {
                    let cmd = command.clone();
                    // Find the preexec's cwd before removing it.
                    let preexec_cwd = result.iter().find_map(|e| match e {
                        Event::ShellCommand {
                            command: c,
                            exit_status: None,
                            cwd,
                            ..
                        } if *c == cmd => Some(cwd.clone()),
                        _ => None,
                    });
                    result.retain(|e| {
                        !matches!(
                            e,
                            Event::ShellCommand {
                                command: c,
                                exit_status: None,
                                ..
                            } if *c == cmd
                        )
                    });
                    // If we found a preexec, use its cwd on the merged event.
                    let mut event = event;
                    if let Some(cwd) = preexec_cwd
                        && let Event::ShellCommand {
                            cwd: ref mut event_cwd,
                            ..
                        } = event
                    {
                        *event_cwd = cwd;
                    }
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
    let dedup_window = chrono::Duration::milliseconds(CROSS_TYPE_DEDUP_WINDOW_MS);

    // Collect browser selection timestamps and normalized plain texts for matching.
    let browser_entries: Vec<(DateTime<Utc>, String)> = events
        .iter()
        .filter_map(|e| match e {
            Event::BrowserSelection {
                timestamp,
                plain_text,
                text,
                ..
            } => {
                // Use plain_text for comparison when available (more reliable
                // for rich-text selections). Fall back to text for old events.
                let compare = if plain_text.is_empty() {
                    text
                } else {
                    plain_text
                };
                Some((*timestamp, normalize_text(compare)))
            }
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
        let norm = normalize_text(text);
        // Drop if any BrowserSelection has matching normalized text within the window.
        !browser_entries.iter().any(|(bs_ts, bs_text)| {
            *bs_text == norm && (*timestamp - *bs_ts).abs().le(&dedup_window)
        })
    });
}

/// Normalize text for comparison: collapse all whitespace to single spaces, trim.
pub(crate) fn normalize_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Drop text `ClipboardSelection` events whose normalized text matches any
/// `ExternalSelection` or `BrowserSelection` (via `plain_text`) in the list.
///
/// Image clipboard events are never deduped by this pass.
fn dedup_clipboard_selections(events: &mut Vec<Event>) {
    // Collect normalized texts from richer sources, excluding empty strings
    // (whitespace-only text normalizes to "" and would vacuously match
    // any other whitespace-only content).
    let richer_texts: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::ExternalSelection { text, .. } => Some(normalize_text(text)),
            Event::BrowserSelection { plain_text, .. } => Some(normalize_text(plain_text)),
            _ => None,
        })
        .filter(|t| !t.is_empty())
        .collect();

    if richer_texts.is_empty() {
        return;
    }

    events.retain(|e| {
        let Event::ClipboardSelection {
            content: ClipboardContent::Text { text },
            ..
        } = e
        else {
            return true;
        };
        let norm = normalize_text(text);
        // Empty normalized text (whitespace-only clipboard) cannot meaningfully
        // match a richer source: skip dedup to avoid vacuous matches.
        if norm.is_empty() {
            return true;
        }
        !richer_texts.contains(&norm)
    });
}

// ── Run orchestrator ─────────────────────────────────────────────────────────

/// Process a single non-Words run through the composable per-run
/// transformations: cursor compression, snapshot union, diff net-change,
/// and selection/command collapse with cross-type dedup.
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
                last_seen,
                files,
                regions,
            } => {
                snapshots.push((timestamp, last_seen, files, regions));
            }
            Event::FileDiff {
                timestamp,
                path,
                old,
                new,
            } => {
                diffs.push((timestamp, path, old, new));
            }
            Event::ExternalSelection { .. }
            | Event::BrowserSelection { .. }
            | Event::ClipboardSelection { .. } => {
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

    if let Some((timestamp, last_seen, files, regions)) = merged_snap
        && !regions.is_empty()
    {
        result.push(Event::EditorSnapshot {
            timestamp,
            last_seen,
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

/// Sort events chronologically, apply global progressive selection
/// subsumption, then process all non-speech runs in a single pass:
/// compress cursor-only snapshots, union adjacent snapshots, merge
/// same-path diffs into net-change events, collapse selections and
/// commands, and drop trailing cursors when speech is present.
///
/// Mutates `events` in place. Path-format-agnostic (works with both
/// absolute and relative paths).
pub fn compress_and_merge(events: &mut Vec<Event>) {
    // Step 1: chronological sort.
    events.sort_by_key(|a| a.timestamp());

    // Step 2: global progressive selection subsumption.
    // Must run before run-splitting so intermediate events serve as temporal
    // bridges for chain subsumption (A ⊂ B ⊂ C across run boundaries).
    subsume_progressive_selections(events);

    // Step 3: single pass — split on Words boundaries, process each run.
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

    // Step 4: global clipboard dedup — drop text clipboard events whose
    // normalized text matches any ExternalSelection or BrowserSelection
    // across the entire output (not just within each run).
    dedup_clipboard_selections(&mut output);

    // Step 5: drop trailing cursor-only snapshot when speech is present.
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
