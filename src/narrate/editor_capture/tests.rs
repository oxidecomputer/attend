use super::*;
use crate::clock::MockClock;
use crate::narrate::capture::CaptureControl;
use crate::narrate::merge::Event;
use crate::state::{EditorState, FileEntry, Position, Selection};
use camino::Utf8PathBuf;
use chrono::Duration;
use std::sync::{Arc, Mutex};

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

// ---------------------------------------------------------------------------
// Integration tests: drive the full polling loop via MockClock + stub source
// ---------------------------------------------------------------------------

/// Stub editor source that returns a shared, externally-controlled state.
///
/// The test harness sets the `Mutex<Option<EditorState>>` before advancing
/// the clock; the polling thread reads it on each tick.
struct SequencedEditorSource {
    state: Arc<Mutex<Option<EditorState>>>,
}

impl EditorStateSource for SequencedEditorSource {
    fn current(
        &self,
        _cwd: Option<&Utf8Path>,
        _include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Option<EditorState>> {
        Ok(self.state.lock().unwrap().clone())
    }
}

/// Helper: count only `EditorSnapshot` events in the events vec.
fn snapshot_count(events: &Mutex<Vec<Event>>) -> usize {
    events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| matches!(e, Event::EditorSnapshot { .. }))
        .count()
}

/// Helper: extract `files` from all `EditorSnapshot` events.
fn snapshot_files(events: &Mutex<Vec<Event>>) -> Vec<Vec<FileEntry>> {
    events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match e {
            Event::EditorSnapshot { files, .. } => Some(files.clone()),
            _ => None,
        })
        .collect()
}

/// Cursor-only state emits exactly one EditorSnapshot after the dwell
/// timeout elapses when driven through the full polling loop.
///
/// Invariant: a cursor-only snapshot that persists for longer than
/// CURSOR_DWELL_MS (500ms) produces exactly one EditorSnapshot event.
#[test]
fn integration_cursor_dwell_fires_after_timeout() {
    let clock = MockClock::new(Utc::now());
    let control = Arc::new(CaptureControl::new());
    let events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let open_paths: Arc<Mutex<Vec<Utf8PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let state: Arc<Mutex<Option<EditorState>>> = Arc::new(Mutex::new(None));

    let source = Box::new(SequencedEditorSource {
        state: Arc::clone(&state),
    });

    let handle = spawn(
        source,
        Arc::new(clock.clone()),
        Arc::clone(&control),
        None,
        Arc::clone(&events),
        Arc::clone(&open_paths),
    );

    // Wait for the polling thread to enter its first sleep.
    clock.wait_for_sleepers(1);

    // Set a cursor-only state.
    *state.lock().unwrap() = Some(EditorState {
        files: vec![cursor_entry("foo.rs", 10, 5)],
        cwd: None,
    });

    // Advance through several poll ticks but stay under the dwell timeout.
    // Each tick is 100ms; dwell is 500ms. 3 ticks = 300ms: not enough.
    for _ in 0..3 {
        clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));
    }
    assert_eq!(
        snapshot_count(&events),
        0,
        "no snapshot before dwell timeout"
    );

    // Advance past the dwell timeout. We need enough ticks for the
    // cursor to have been pending for >= 500ms total.
    // We've already advanced 300ms. 3 more ticks = 300ms more = 600ms total.
    for _ in 0..3 {
        clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));
    }

    assert_eq!(
        snapshot_count(&events),
        1,
        "exactly one snapshot after dwell timeout"
    );

    let files = snapshot_files(&events);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0], vec![cursor_entry("foo.rs", 10, 5)]);

    // Stop and join.
    control.stop();
    clock.advance(std::time::Duration::from_millis(EDITOR_POLL_MS));
    handle.join().unwrap();
}

/// Changing the cursor position before the dwell timeout resets the timer,
/// so no EditorSnapshot is emitted.
///
/// Invariant: if the cursor moves to a new position before CURSOR_DWELL_MS
/// elapses, the original pending snapshot is discarded and the timer restarts.
#[test]
fn integration_cursor_replaced_before_dwell() {
    let clock = MockClock::new(Utc::now());
    let control = Arc::new(CaptureControl::new());
    let events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let open_paths: Arc<Mutex<Vec<Utf8PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let state: Arc<Mutex<Option<EditorState>>> = Arc::new(Mutex::new(None));

    let source = Box::new(SequencedEditorSource {
        state: Arc::clone(&state),
    });

    let handle = spawn(
        source,
        Arc::new(clock.clone()),
        Arc::clone(&control),
        None,
        Arc::clone(&events),
        Arc::clone(&open_paths),
    );

    clock.wait_for_sleepers(1);

    // Set cursor at position A.
    *state.lock().unwrap() = Some(EditorState {
        files: vec![cursor_entry("foo.rs", 1, 1)],
        cwd: None,
    });

    // Advance 2 ticks (200ms): cursor A is pending.
    for _ in 0..2 {
        clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));
    }
    assert_eq!(snapshot_count(&events), 0);

    // Switch to cursor at position B: resets the dwell timer.
    *state.lock().unwrap() = Some(EditorState {
        files: vec![cursor_entry("foo.rs", 50, 10)],
        cwd: None,
    });

    // Advance 3 more ticks (300ms total since B appeared).
    // This is 500ms total wall time but only 300ms since cursor B,
    // so B should NOT have dwelled yet.
    for _ in 0..3 {
        clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));
    }
    assert_eq!(
        snapshot_count(&events),
        0,
        "no snapshot: cursor B hasn't dwelled long enough"
    );

    // Stop and join.
    control.stop();
    clock.advance(std::time::Duration::from_millis(EDITOR_POLL_MS));
    handle.join().unwrap();
}

/// A real selection arriving while a cursor-only snapshot is pending emits
/// the selection immediately and discards the pending cursor.
///
/// Invariant: selections with non-cursor-like ranges bypass dwell filtering
/// and emit immediately. Any pending cursor-only snapshot is discarded, so
/// only the selection event appears.
#[test]
fn integration_selection_interrupts_cursor_dwell() {
    let clock = MockClock::new(Utc::now());
    let control = Arc::new(CaptureControl::new());
    let events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let open_paths: Arc<Mutex<Vec<Utf8PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let state: Arc<Mutex<Option<EditorState>>> = Arc::new(Mutex::new(None));

    let source = Box::new(SequencedEditorSource {
        state: Arc::clone(&state),
    });

    let handle = spawn(
        source,
        Arc::new(clock.clone()),
        Arc::clone(&control),
        None,
        Arc::clone(&events),
        Arc::clone(&open_paths),
    );

    clock.wait_for_sleepers(1);

    // Set cursor-only state to start a dwell timer.
    *state.lock().unwrap() = Some(EditorState {
        files: vec![cursor_entry("foo.rs", 1, 1)],
        cwd: None,
    });

    // Advance 2 ticks (200ms): cursor is pending.
    for _ in 0..2 {
        clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));
    }
    assert_eq!(snapshot_count(&events), 0, "cursor still pending");

    // Replace with a real selection.
    let sel_files = vec![selection_entry("foo.rs", 1, 1, 5, 20)];
    *state.lock().unwrap() = Some(EditorState {
        files: sel_files.clone(),
        cwd: None,
    });

    // Advance 1 tick: the selection should emit immediately.
    clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));

    assert_eq!(
        snapshot_count(&events),
        1,
        "exactly one snapshot: the selection"
    );

    let files = snapshot_files(&events);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0], sel_files, "emitted event contains the selection");

    // Advance well past the original dwell timeout: no cursor snapshot fires.
    for _ in 0..6 {
        clock.advance_and_settle(std::time::Duration::from_millis(EDITOR_POLL_MS));
    }

    // The selection events after the first are Extend (same state), not new
    // snapshots. Only 1 EditorSnapshot total.
    assert_eq!(
        snapshot_count(&events),
        1,
        "no extra snapshot from discarded cursor"
    );

    // Stop and join.
    control.stop();
    clock.advance(std::time::Duration::from_millis(EDITOR_POLL_MS));
    handle.join().unwrap();
}
