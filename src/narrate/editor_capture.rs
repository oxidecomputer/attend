//! Background polling of editor selections.
//!
//! Spawns a thread that polls an [`EditorStateSource`] every 100ms and emits
//! [`Event::EditorSnapshot`] whenever the file/selection set changes.
//!
//! Cursor dwell logic is encapsulated in [`DwellTracker`]: cursor-only
//! snapshots are deferred until the cursor rests for a configurable
//! duration, while real selections emit immediately.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, TimeDelta, Utc};

use super::merge::Event;
use crate::clock::Clock;
use crate::state::{self, EditorState};
use crate::view;

/// Source of editor state snapshots.
///
/// Abstracts the platform-specific editor query so tests can substitute
/// a stub that returns scripted file lists and cursor positions.
/// The production implementation calls [`EditorState::current`].
pub trait EditorStateSource: Send {
    /// Query the current editor state.
    fn current(
        &self,
        cwd: Option<&Utf8Path>,
        include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Option<EditorState>>;
}

/// Production implementation: queries real editor(s) via platform APIs.
pub(crate) struct RealEditorSource;

impl EditorStateSource for RealEditorSource {
    fn current(
        &self,
        cwd: Option<&Utf8Path>,
        include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Option<EditorState>> {
        EditorState::current(cwd, include_dirs)
    }
}

/// How often to poll for editor selection changes (ms).
const EDITOR_POLL_MS: u64 = 100;

/// Minimum dwell time before emitting a cursor-only snapshot (ms).
///
/// Rapid file scanning generates many low-value cursor positions. We only
/// emit a cursor-only snapshot once the cursor has rested at a position for
/// at least this long. Snapshots with real selections (highlights) are
/// emitted immediately since they indicate intentional pointing.
const CURSOR_DWELL_MS: u64 = 500;

/// Result of processing an editor state update through the dwell tracker.
#[derive(Debug)]
enum EditorUpdate {
    /// Changed state with real selections: emit immediately.
    Emit(Vec<state::FileEntry>),
    /// Unchanged state with real selections: extend last event's `last_seen`.
    Extend,
    /// Cursor-only or truly no change: pending/skip.
    None,
}

/// Pure state machine for cursor dwell filtering.
///
/// Tracks previous editor state and a pending cursor-only snapshot. The
/// caller drives the tracker via [`tick`] (time-based flush) and [`update`]
/// (new editor state). When either method returns `Some`, the caller should
/// emit that snapshot.
struct DwellTracker {
    dwell_duration: TimeDelta,
    /// Last emitted-or-stored file state, used for deduplication.
    prev_files: Option<Vec<state::FileEntry>>,
    /// A cursor-only snapshot waiting for the dwell timeout.
    pending_cursor: Option<(DateTime<Utc>, Vec<state::FileEntry>)>,
}

impl DwellTracker {
    /// Create a new tracker with the given dwell duration.
    fn new(dwell_duration: TimeDelta) -> Self {
        Self {
            dwell_duration,
            prev_files: None,
            pending_cursor: None,
        }
    }

    /// Check whether a pending cursor-only snapshot has dwelled long enough.
    ///
    /// Returns `Some(files)` if the pending snapshot should be emitted now,
    /// clearing the pending state. Returns `None` otherwise.
    fn tick(&mut self, now: DateTime<Utc>) -> Option<Vec<state::FileEntry>> {
        if let Some((changed_at, _)) = &self.pending_cursor
            && (now - *changed_at) >= self.dwell_duration
        {
            return self.pending_cursor.take().map(|(_, files)| files);
        }
        None
    }

    /// Process a new editor state snapshot.
    ///
    /// Returns `Emit(files)` if the snapshot should be emitted immediately
    /// (i.e. it contains real selections and is different from previous).
    /// Returns `Extend` if unchanged but has real selections (for `last_seen`).
    /// Cursor-only snapshots are stored as pending and will be emitted later
    /// via [`tick`]. Returns `None` for cursor-only or truly unchanged cursors.
    fn update(&mut self, files: Vec<state::FileEntry>, now: DateTime<Utc>) -> EditorUpdate {
        // Check if unchanged from last state.
        if self.prev_files.as_ref() == Some(&files) {
            // If the unchanged state has real selections, signal Extend
            // so the caller can update last_seen. Cursor-only duplicates
            // are just skipped.
            let has_real_selection = files
                .iter()
                .any(|f| f.selections.iter().any(|s| !s.is_cursor_like()));
            if has_real_selection {
                return EditorUpdate::Extend;
            }
            return EditorUpdate::None;
        }

        self.prev_files = Some(files.clone());

        let cursor_only = files
            .iter()
            .all(|f| f.selections.iter().all(|s| s.is_cursor_like()));

        if cursor_only {
            // Defer emission until the cursor dwells at this position.
            self.pending_cursor = Some((now, files));
            EditorUpdate::None
        } else {
            // Real selections are always intentional: emit immediately
            // and discard any pending cursor-only snapshot.
            self.pending_cursor = None;
            EditorUpdate::Emit(files)
        }
    }
}

/// Spawn the editor polling thread.
///
/// Returns the join handle. The thread pushes `EditorSnapshot` events into
/// `events` until stopped via `control`. It also publishes the current set
/// of open file paths into `open_paths` for the diff capture thread to read.
pub(super) fn spawn(
    source: Box<dyn EditorStateSource>,
    clock: Arc<dyn Clock>,
    control: Arc<super::capture::CaptureControl>,
    cwd: Option<Utf8PathBuf>,
    events: Arc<Mutex<Vec<Event>>>,
    open_paths: Arc<Mutex<Vec<Utf8PathBuf>>>,
) -> std::thread::JoinHandle<()> {
    crate::clock::spawn_clock_thread("editor", &*clock, move |clock| {
        let mut lang_cache = view::LanguageCache::new();
        let mut tracker = DwellTracker::new(TimeDelta::milliseconds(CURSOR_DWELL_MS as i64));

        loop {
            if control.wait_while_paused(&*clock) {
                break;
            }

            clock.sleep(Duration::from_millis(EDITOR_POLL_MS));

            let now = clock.now();

            // Flush a dwelled cursor-only snapshot if enough time has passed.
            if let Some(files) = tracker.tick(now) {
                let timestamp = clock.now();
                let regions = capture_snapshot_regions(&files, None, &mut lang_cache);
                events.lock().unwrap().push(Event::EditorSnapshot {
                    timestamp,
                    last_seen: timestamp,
                    files,
                    regions,
                });
            }

            let state = match source.current(cwd.as_deref(), &[]) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            // Publish open file paths for the diff capture thread.
            *open_paths.lock().unwrap() = state.files.iter().map(|f| f.path.clone()).collect();

            match tracker.update(state.files, now) {
                EditorUpdate::Emit(files) => {
                    let timestamp = clock.now();
                    let regions = capture_snapshot_regions(&files, None, &mut lang_cache);
                    events.lock().unwrap().push(Event::EditorSnapshot {
                        timestamp,
                        last_seen: timestamp,
                        files,
                        regions,
                    });
                }
                EditorUpdate::Extend => {
                    let now_utc = clock.now();
                    let mut guard = events.lock().unwrap();
                    if let Some(Event::EditorSnapshot { last_seen, .. }) = guard.last_mut() {
                        *last_seen = now_utc;
                    }
                }
                EditorUpdate::None => {}
            }
        }
    })
}

/// Capture file regions for an editor snapshot.
fn capture_snapshot_regions(
    files: &[state::FileEntry],
    cwd: Option<&Utf8Path>,
    lang_cache: &mut view::LanguageCache,
) -> Vec<view::CapturedRegion> {
    let extent = view::Extent::Lines {
        before: 1,
        after: 1,
    };
    view::capture_regions(files, cwd, extent, lang_cache).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FileEntry, Position, Selection};
    use camino::Utf8PathBuf;
    use chrono::Duration;

    /// Build a cursor-only FileEntry (zero-width cursor at line:col).
    fn cursor_entry(path: &str, line: usize, col: usize) -> FileEntry {
        let pos = Position::of(line, col).expect("valid position");
        FileEntry {
            path: Utf8PathBuf::from(path),
            selections: vec![Selection {
                start: pos,
                end: pos,
            }],
        }
    }

    /// Build a FileEntry with a real multi-character selection.
    fn selection_entry(
        path: &str,
        start_line: usize,
        start_col: usize,
        end_line: usize,
        end_col: usize,
    ) -> FileEntry {
        FileEntry {
            path: Utf8PathBuf::from(path),
            selections: vec![Selection {
                start: Position::of(start_line, start_col).expect("valid start"),
                end: Position::of(end_line, end_col).expect("valid end"),
            }],
        }
    }

    /// Cursor-only updates are deferred; they emit only after the dwell
    /// timeout elapses.
    #[test]
    fn cursor_only_deferred() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let files = vec![cursor_entry("foo.rs", 1, 1)];

        // update returns None for cursor-only state.
        assert!(matches!(
            tracker.update(files.clone(), now),
            EditorUpdate::None
        ));

        // tick before dwell timeout returns None.
        assert_eq!(tracker.tick(now + dwell / 2), None);

        // tick after dwell timeout returns the deferred files.
        let emitted = tracker.tick(now + dwell);
        assert_eq!(emitted, Some(files));

        // Subsequent tick returns None (pending was consumed).
        assert_eq!(tracker.tick(now + dwell * 2), None);
    }

    /// Updates with real selections (non-cursor-like) emit immediately.
    #[test]
    fn selection_immediate() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let files = vec![selection_entry("foo.rs", 1, 1, 1, 10)];

        let result = tracker.update(files.clone(), now);
        assert!(matches!(result, EditorUpdate::Emit(ref f) if f == &files));
    }

    /// Rapid cursor-only changes before the dwell timeout: only the last
    /// position is emitted once the timeout elapses.
    #[test]
    fn rapid_cursor_changes() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let first = vec![cursor_entry("foo.rs", 1, 1)];
        let second = vec![cursor_entry("foo.rs", 5, 1)];
        let third = vec![cursor_entry("foo.rs", 10, 1)];

        // Each cursor-only update returns None and replaces the pending.
        assert!(matches!(tracker.update(first, now), EditorUpdate::None));
        assert!(matches!(
            tracker.update(second, now + Duration::milliseconds(30)),
            EditorUpdate::None
        ));
        assert!(matches!(
            tracker.update(third.clone(), now + Duration::milliseconds(60)),
            EditorUpdate::None
        ));

        // tick before dwell from the *last* update returns None.
        assert_eq!(tracker.tick(now + Duration::milliseconds(60 + 50)), None);

        // tick after dwell from the last update returns only the last files.
        let emitted = tracker.tick(now + Duration::milliseconds(60 + 100));
        assert_eq!(emitted, Some(third));
    }

    /// A pending cursor-only snapshot is discarded when a selection arrives
    /// before the dwell timeout.
    #[test]
    fn cursor_then_selection() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let cursor = vec![cursor_entry("foo.rs", 1, 1)];
        let sel = vec![selection_entry("foo.rs", 1, 1, 1, 10)];

        // Cursor-only: deferred.
        assert!(matches!(tracker.update(cursor, now), EditorUpdate::None));

        // Selection arrives before dwell timeout: emits immediately.
        let result = tracker.update(sel.clone(), now + Duration::milliseconds(50));
        assert!(matches!(result, EditorUpdate::Emit(ref f) if f == &sel));

        // tick after original dwell timeout returns None (cursor was discarded).
        assert_eq!(tracker.tick(now + dwell * 2), None);
    }

    /// Updating with the same file state containing real selections returns
    /// Extend (for last_seen tracking), not None.
    #[test]
    fn unchanged_selection_state_extends() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let files = vec![selection_entry("foo.rs", 1, 1, 1, 10)];

        // First update emits.
        assert!(matches!(
            tracker.update(files.clone(), now),
            EditorUpdate::Emit(_)
        ));

        // Same state again with real selections: Extend.
        assert!(matches!(
            tracker.update(files, now + Duration::milliseconds(50)),
            EditorUpdate::Extend
        ));
    }

    /// Updating with the same cursor-only file state returns None (not Extend).
    #[test]
    fn unchanged_cursor_state() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let files = vec![cursor_entry("foo.rs", 1, 1)];

        // First update: pending (cursor-only).
        assert!(matches!(
            tracker.update(files.clone(), now),
            EditorUpdate::None
        ));

        // Same cursor-only state: still None.
        assert!(matches!(
            tracker.update(files, now + Duration::milliseconds(50)),
            EditorUpdate::None
        ));
    }

    /// After a selection emits immediately, a subsequent cursor-only update
    /// starts a fresh dwell period.
    #[test]
    fn selection_then_cursor() {
        let dwell = Duration::milliseconds(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Utc::now();

        let sel = vec![selection_entry("foo.rs", 1, 1, 1, 10)];
        let cursor = vec![cursor_entry("foo.rs", 5, 1)];

        // Selection emits immediately.
        assert!(matches!(
            tracker.update(sel.clone(), now),
            EditorUpdate::Emit(ref f) if f == &sel
        ));

        // Cursor-only: deferred.
        let t1 = now + Duration::milliseconds(200);
        assert!(matches!(
            tracker.update(cursor.clone(), t1),
            EditorUpdate::None
        ));

        // tick before dwell from cursor update returns None.
        assert_eq!(tracker.tick(t1 + dwell / 2), None);

        // tick after dwell from cursor update returns the cursor files.
        let emitted = tracker.tick(t1 + dwell);
        assert_eq!(emitted, Some(cursor));
    }
}
