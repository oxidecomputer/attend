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

    let emitted = tracker.update(snap.clone(), now);
    assert_eq!(emitted, Some(snap));
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
    assert_eq!(tracker.update(empty, now), None);

    let blank = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("".into()),
    };
    assert_eq!(tracker.update(blank, now), None);
}

/// Same text as previous emission is deduplicated.
#[test]
fn dedup_same_text() {
    let mut tracker = ExtDwellTracker::new();
    let now = Instant::now();

    let snap = ExternalSnapshot {
        app: "iTerm2".into(),
        window_title: "~/src".into(),
        selected_text: Some("hello".into()),
    };

    // First: emitted.
    assert!(tracker.update(snap.clone(), now).is_some());

    // Same text again: suppressed.
    assert_eq!(tracker.update(snap, now + Duration::from_millis(50)), None);
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

    assert!(tracker.update(snap1, now).is_some());
    assert!(
        tracker
            .update(snap2, now + Duration::from_millis(30))
            .is_some()
    );
    assert!(
        tracker
            .update(snap3, now + Duration::from_millis(60))
            .is_some()
    );
}
