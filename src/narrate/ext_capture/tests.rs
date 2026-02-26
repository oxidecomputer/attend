use super::*;

/// Selections are emitted immediately on change.
#[test]
fn selection_emitted_immediately() {
    let mut tracker = ExtDwellTracker::new();
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    let result = tracker.update(snap.clone(), now);
    assert!(matches!(result, ExtUpdate::New(ref s) if s == &snap));
}

/// Empty or no selection returns None.
#[test]
fn empty_selection_returns_none() {
    let mut tracker = ExtDwellTracker::new();
    let now = Instant::now();

    let empty = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: None,
    };
    assert!(matches!(tracker.update(empty, now), ExtUpdate::None));

    let blank = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("".into()),
    };
    assert!(matches!(tracker.update(blank, now), ExtUpdate::None));
}

/// Same text as previous emission returns Extend (for last_seen tracking).
#[test]
fn dedup_same_text() {
    let mut tracker = ExtDwellTracker::new();
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    // First: emitted as New.
    assert!(matches!(
        tracker.update(snap.clone(), now),
        ExtUpdate::New(_)
    ));

    // Same text again: Extend (not None).
    assert!(matches!(
        tracker.update(snap, now + Duration::from_millis(50)),
        ExtUpdate::Extend
    ));
}

/// Same text as previous emission returns Extend (for last_seen update).
#[test]
fn same_text_returns_extend() {
    let mut tracker = ExtDwellTracker::new();
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    // First: emitted as New.
    assert!(matches!(
        tracker.update(snap.clone(), now),
        ExtUpdate::New(_)
    ));

    // Same text again: Extend (not None).
    assert!(matches!(
        tracker.update(snap, now + Duration::from_millis(50)),
        ExtUpdate::Extend
    ));
}

/// Different texts are all emitted.
#[test]
fn different_texts_all_emitted() {
    let mut tracker = ExtDwellTracker::new();
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

    assert!(matches!(tracker.update(snap1, now), ExtUpdate::New(_)));
    assert!(matches!(
        tracker.update(snap2, now + Duration::from_millis(30)),
        ExtUpdate::New(_)
    ));
    assert!(matches!(
        tracker.update(snap3, now + Duration::from_millis(60)),
        ExtUpdate::New(_)
    ));
}
