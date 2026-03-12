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
