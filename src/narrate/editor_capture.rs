//! Background polling of editor selections.
//!
//! Spawns a thread that polls [`EditorState`] every 100ms and emits
//! [`Event::EditorSnapshot`] whenever the file/selection set changes.
//!
//! Cursor dwell logic is encapsulated in [`DwellTracker`]: cursor-only
//! snapshots are deferred until the cursor rests for a configurable
//! duration, while real selections emit immediately.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};

use super::merge::Event;
use crate::state::{self, EditorState};
use crate::view;

/// How often to poll for editor selection changes (ms).
const EDITOR_POLL_MS: u64 = 100;

/// Minimum dwell time before emitting a cursor-only snapshot (ms).
///
/// Rapid file scanning generates many low-value cursor positions. We only
/// emit a cursor-only snapshot once the cursor has rested at a position for
/// at least this long. Snapshots with real selections (highlights) are
/// emitted immediately since they indicate intentional pointing.
const CURSOR_DWELL_MS: u64 = 500;

/// Pure state machine for cursor dwell filtering.
///
/// Tracks previous editor state and a pending cursor-only snapshot. The
/// caller drives the tracker via [`tick`] (time-based flush) and [`update`]
/// (new editor state). When either method returns `Some`, the caller should
/// emit that snapshot.
struct DwellTracker {
    dwell_duration: Duration,
    /// Last emitted-or-stored file state, used for deduplication.
    prev_files: Option<Vec<state::FileEntry>>,
    /// A cursor-only snapshot waiting for the dwell timeout.
    pending_cursor: Option<(Instant, Vec<state::FileEntry>)>,
}

impl DwellTracker {
    /// Create a new tracker with the given dwell duration.
    fn new(dwell_duration: Duration) -> Self {
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
    fn tick(&mut self, now: Instant) -> Option<Vec<state::FileEntry>> {
        if let Some((changed_at, _)) = &self.pending_cursor
            && now.duration_since(*changed_at) >= self.dwell_duration
        {
            return self.pending_cursor.take().map(|(_, files)| files);
        }
        None
    }

    /// Process a new editor state snapshot.
    ///
    /// Returns `Some(files)` if the snapshot should be emitted immediately
    /// (i.e. it contains real selections). Cursor-only snapshots are stored
    /// as pending and will be emitted later via [`tick`]. Unchanged state
    /// is deduplicated and returns `None`.
    fn update(
        &mut self,
        files: Vec<state::FileEntry>,
        now: Instant,
    ) -> Option<Vec<state::FileEntry>> {
        // Deduplicate: if unchanged from last state, skip.
        if self.prev_files.as_ref() == Some(&files) {
            return None;
        }

        self.prev_files = Some(files.clone());

        let cursor_only = files
            .iter()
            .all(|f| f.selections.iter().all(|s| s.is_cursor_like()));

        if cursor_only {
            // Defer emission until the cursor dwells at this position.
            self.pending_cursor = Some((now, files));
            None
        } else {
            // Real selections are always intentional: emit immediately
            // and discard any pending cursor-only snapshot.
            self.pending_cursor = None;
            Some(files)
        }
    }
}

/// Spawn the editor polling thread.
///
/// Returns the join handle. The thread pushes `EditorSnapshot` events into
/// `events` until `stop` is set. It also publishes the current set of open
/// file paths into `open_paths` for the diff capture thread to read.
pub(super) fn spawn(
    stop: Arc<AtomicBool>,
    cwd: Option<Utf8PathBuf>,
    events: Arc<Mutex<Vec<Event>>>,
    open_paths: Arc<Mutex<Vec<Utf8PathBuf>>>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut tracker = DwellTracker::new(Duration::from_millis(CURSOR_DWELL_MS));

        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(EDITOR_POLL_MS));

            let now = Instant::now();

            // Flush a dwelled cursor-only snapshot if enough time has passed.
            if let Some(files) = tracker.tick(now) {
                let timestamp = chrono::Utc::now();
                let regions = capture_snapshot_regions(&files, None);
                events.lock().unwrap().push(Event::EditorSnapshot {
                    timestamp,
                    files,
                    regions,
                });
            }

            let state = match EditorState::current(cwd.as_deref(), &[]) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            // Publish open file paths for the diff capture thread.
            *open_paths.lock().unwrap() = state.files.iter().map(|f| f.path.clone()).collect();

            if let Some(files) = tracker.update(state.files, now) {
                let timestamp = chrono::Utc::now();
                let regions = capture_snapshot_regions(&files, None);
                events.lock().unwrap().push(Event::EditorSnapshot {
                    timestamp,
                    files,
                    regions,
                });
            }
        }
    })
}

/// Capture file regions for an editor snapshot.
fn capture_snapshot_regions(
    files: &[state::FileEntry],
    cwd: Option<&Utf8Path>,
) -> Vec<view::CapturedRegion> {
    let extent = view::Extent::Lines {
        before: 1,
        after: 1,
    };
    view::capture_regions(files, cwd, extent).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{FileEntry, Position, Selection};
    use camino::Utf8PathBuf;

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
        let dwell = Duration::from_millis(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Instant::now();

        let files = vec![cursor_entry("foo.rs", 1, 1)];

        // update returns None for cursor-only state.
        assert_eq!(tracker.update(files.clone(), now), None);

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
        let dwell = Duration::from_millis(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Instant::now();

        let files = vec![selection_entry("foo.rs", 1, 1, 1, 10)];

        let emitted = tracker.update(files.clone(), now);
        assert_eq!(emitted, Some(files));
    }

    /// Rapid cursor-only changes before the dwell timeout: only the last
    /// position is emitted once the timeout elapses.
    #[test]
    fn rapid_cursor_changes() {
        let dwell = Duration::from_millis(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Instant::now();

        let first = vec![cursor_entry("foo.rs", 1, 1)];
        let second = vec![cursor_entry("foo.rs", 5, 1)];
        let third = vec![cursor_entry("foo.rs", 10, 1)];

        // Each cursor-only update returns None and replaces the pending.
        assert_eq!(tracker.update(first, now), None);
        assert_eq!(
            tracker.update(second, now + Duration::from_millis(30)),
            None
        );
        assert_eq!(
            tracker.update(third.clone(), now + Duration::from_millis(60)),
            None
        );

        // tick before dwell from the *last* update returns None.
        assert_eq!(tracker.tick(now + Duration::from_millis(60 + 50)), None);

        // tick after dwell from the last update returns only the last files.
        let emitted = tracker.tick(now + Duration::from_millis(60 + 100));
        assert_eq!(emitted, Some(third));
    }

    /// A pending cursor-only snapshot is discarded when a selection arrives
    /// before the dwell timeout.
    #[test]
    fn cursor_then_selection() {
        let dwell = Duration::from_millis(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Instant::now();

        let cursor = vec![cursor_entry("foo.rs", 1, 1)];
        let sel = vec![selection_entry("foo.rs", 1, 1, 1, 10)];

        // Cursor-only: deferred.
        assert_eq!(tracker.update(cursor, now), None);

        // Selection arrives before dwell timeout: emits immediately.
        let emitted = tracker.update(sel.clone(), now + Duration::from_millis(50));
        assert_eq!(emitted, Some(sel));

        // tick after original dwell timeout returns None (cursor was discarded).
        assert_eq!(tracker.tick(now + dwell * 2), None);
    }

    /// Updating with the same file state is a no-op (deduplicated).
    #[test]
    fn unchanged_state() {
        let dwell = Duration::from_millis(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Instant::now();

        let files = vec![selection_entry("foo.rs", 1, 1, 1, 10)];

        // First update emits.
        assert_eq!(tracker.update(files.clone(), now), Some(files.clone()));

        // Same state again: returns None.
        assert_eq!(tracker.update(files, now + Duration::from_millis(50)), None);
    }

    /// After a selection emits immediately, a subsequent cursor-only update
    /// starts a fresh dwell period.
    #[test]
    fn selection_then_cursor() {
        let dwell = Duration::from_millis(100);
        let mut tracker = DwellTracker::new(dwell);
        let now = Instant::now();

        let sel = vec![selection_entry("foo.rs", 1, 1, 1, 10)];
        let cursor = vec![cursor_entry("foo.rs", 5, 1)];

        // Selection emits immediately.
        assert_eq!(tracker.update(sel.clone(), now), Some(sel));

        // Cursor-only: deferred.
        let t1 = now + Duration::from_millis(200);
        assert_eq!(tracker.update(cursor.clone(), t1), None);

        // tick before dwell from cursor update returns None.
        assert_eq!(tracker.tick(t1 + dwell / 2), None);

        // tick after dwell from cursor update returns the cursor files.
        let emitted = tracker.tick(t1 + dwell);
        assert_eq!(emitted, Some(cursor));
    }
}
