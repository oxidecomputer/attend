use super::*;

/// Selections are deferred until the dwell timeout elapses.
#[test]
fn selection_deferred() {
    let dwell = Duration::from_millis(100);
    let mut tracker = ExtDwellTracker::new(dwell);
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    // update defers the selection.
    tracker.update(snap.clone(), now);

    // tick before dwell timeout returns None.
    assert_eq!(tracker.tick(now + dwell / 2), None);

    // tick after dwell timeout returns the snapshot.
    let emitted = tracker.tick(now + dwell);
    assert_eq!(emitted, Some(snap));

    // Subsequent tick returns None (pending consumed).
    assert_eq!(tracker.tick(now + dwell * 2), None);
}

/// Empty or no selection clears pending state.
#[test]
fn empty_selection_clears_pending() {
    let dwell = Duration::from_millis(100);
    let mut tracker = ExtDwellTracker::new(dwell);
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };
    tracker.update(snap, now);

    // Empty selection clears pending.
    let empty = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: None,
    };
    tracker.update(empty, now + Duration::from_millis(50));

    // tick after dwell returns None: pending was cleared.
    assert_eq!(tracker.tick(now + dwell * 2), None);
}

/// Same text as previous emission is deduplicated.
#[test]
fn dedup_same_text() {
    let dwell = Duration::from_millis(100);
    let mut tracker = ExtDwellTracker::new(dwell);
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    // First selection: deferred then emitted.
    tracker.update(snap.clone(), now);
    let emitted = tracker.tick(now + dwell);
    assert!(emitted.is_some());

    // Same text again: should be suppressed.
    tracker.update(snap, now + dwell * 2);
    assert_eq!(tracker.tick(now + dwell * 4), None);
}

/// Rapid selection changes: only the last one survives.
#[test]
fn rapid_changes_keep_last() {
    let dwell = Duration::from_millis(100);
    let mut tracker = ExtDwellTracker::new(dwell);
    let now = Instant::now();

    let snap1 = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("first".into()),
    };
    let snap2 = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("second".into()),
    };
    let snap3 = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("third".into()),
    };

    tracker.update(snap1, now);
    tracker.update(snap2, now + Duration::from_millis(30));
    tracker.update(snap3.clone(), now + Duration::from_millis(60));

    // tick before dwell from last update returns None.
    assert_eq!(tracker.tick(now + Duration::from_millis(60 + 50)), None);

    // tick after dwell from last update returns only the last snapshot.
    let emitted = tracker.tick(now + Duration::from_millis(60 + 100));
    assert_eq!(emitted, Some(snap3));
}

/// Stable pending selection is not replaced by same-text updates.
#[test]
fn same_text_preserves_dwell_timer() {
    let dwell = Duration::from_millis(100);
    let mut tracker = ExtDwellTracker::new(dwell);
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    tracker.update(snap.clone(), now);
    // Same text again: should not reset the dwell timer.
    tracker.update(snap.clone(), now + Duration::from_millis(50));

    // Dwell should fire relative to the first update (t=0), not t=50.
    let emitted = tracker.tick(now + dwell);
    assert_eq!(emitted, Some(snap));
}
