use super::*;

/// Helper: create an ImageData with given dimensions filled with a byte pattern.
fn make_image(width: usize, height: usize, fill: u8) -> ImageData {
    ImageData {
        width,
        height,
        bytes: vec![fill; width * height * 4], // RGBA
    }
}

/// Initializing the tracker with current clipboard content produces no event.
///
/// The tracker seeds with whatever is on the clipboard at start time so that
/// pre-existing content is not reported as a "change."
#[test]
fn seed_does_not_emit() {
    let mut tracker = ClipboardTracker::new_seeded(Some("hello"), None);
    // First check with the same text should not emit (already seeded).
    assert!(matches!(
        tracker.check(Some("hello"), None),
        ClipboardUpdate::Unchanged
    ));
}

/// When clipboard text changes from A to B, a ClipboardContent::Text event is emitted.
#[test]
fn text_change_emits() {
    let mut tracker = ClipboardTracker::new_seeded(Some("hello"), None);
    match tracker.check(Some("world"), None) {
        ClipboardUpdate::Changed(ClipboardContent::Text { text }) => {
            assert_eq!(text, "world");
        }
        other => panic!("expected Changed(Text), got {other:?}"),
    }
}

/// Polling the same text content twice produces only one event.
#[test]
fn same_text_does_not_repeat() {
    let mut tracker = ClipboardTracker::new_seeded(None, None);
    // First change: should emit.
    assert!(matches!(
        tracker.check(Some("hello"), None),
        ClipboardUpdate::Changed(_)
    ));
    // Second check with same text: should not emit.
    assert!(matches!(
        tracker.check(Some("hello"), None),
        ClipboardUpdate::Unchanged
    ));
}

/// Clipboard containing only whitespace/newlines produces no event.
#[test]
fn whitespace_only_skipped() {
    let mut tracker = ClipboardTracker::new_seeded(None, None);
    assert!(matches!(
        tracker.check(Some("   \n\t  "), None),
        ClipboardUpdate::Unchanged
    ));
}

/// Transitioning from empty/error clipboard to text emits.
#[test]
fn empty_to_text_emits() {
    let mut tracker = ClipboardTracker::new_seeded(None, None);
    match tracker.check(Some("hello"), None) {
        ClipboardUpdate::Changed(ClipboardContent::Text { text }) => {
            assert_eq!(text, "hello");
        }
        other => panic!("expected Changed(Text), got {other:?}"),
    }
}

/// Switching from text to image content emits an Image event.
#[test]
fn text_to_image_emits() {
    let mut tracker = ClipboardTracker::new_seeded(Some("hello"), None);
    let img = make_image(10, 10, 0xFF);
    assert!(matches!(
        tracker.check(None, Some(&img)),
        ClipboardUpdate::Changed(ClipboardContent::Image { .. })
    ));
}

/// Switching from image to text content emits a Text event.
#[test]
fn image_to_text_emits() {
    let img = make_image(10, 10, 0xFF);
    let mut tracker = ClipboardTracker::new_seeded(None, Some(&img));
    match tracker.check(Some("hello"), None) {
        ClipboardUpdate::Changed(ClipboardContent::Text { text }) => {
            assert_eq!(text, "hello");
        }
        other => panic!("expected Changed(Text), got {other:?}"),
    }
}

/// Image with different dimensions than previous emits a new event.
#[test]
fn image_dimension_change_emits() {
    let img1 = make_image(10, 10, 0xFF);
    let mut tracker = ClipboardTracker::new_seeded(None, Some(&img1));
    let img2 = make_image(20, 20, 0xFF);
    assert!(matches!(
        tracker.check(None, Some(&img2)),
        ClipboardUpdate::Changed(ClipboardContent::Image { .. })
    ));
}

/// Polling identical image data twice produces only one event.
#[test]
fn same_image_does_not_repeat() {
    let img = make_image(10, 10, 0xAB);
    let mut tracker = ClipboardTracker::new_seeded(None, None);
    // First check: should emit.
    assert!(matches!(
        tracker.check(None, Some(&img)),
        ClipboardUpdate::Changed(_)
    ));
    // Second check with identical image: should not emit.
    let img2 = make_image(10, 10, 0xAB);
    assert!(matches!(
        tracker.check(None, Some(&img2)),
        ClipboardUpdate::Unchanged
    ));
}

/// When both get_text and get_image return nothing, no event is emitted.
#[test]
fn both_unavailable_skips() {
    let mut tracker = ClipboardTracker::new_seeded(None, None);
    assert!(matches!(
        tracker.check(None, None),
        ClipboardUpdate::Unchanged
    ));
}

/// A zero-pixel image (0x0) is still valid image content and emits an event.
///
/// Edge case: the pixel buffer is empty, but blake3 handles empty input
/// correctly (returns the hash of the empty string), so change detection
/// still works.
#[test]
fn empty_image_emits_and_deduplicates() {
    let empty = ImageData {
        width: 0,
        height: 0,
        bytes: vec![],
    };
    let mut tracker = ClipboardTracker::new_seeded(None, None);
    // First check: empty image is new content, should emit.
    assert!(matches!(
        tracker.check(None, Some(&empty)),
        ClipboardUpdate::Changed(ClipboardContent::Image { .. })
    ));
    // Second check: same empty image should not repeat.
    let empty2 = ImageData {
        width: 0,
        height: 0,
        bytes: vec![],
    };
    assert!(matches!(
        tracker.check(None, Some(&empty2)),
        ClipboardUpdate::Unchanged
    ));
}
