//! Background polling of system clipboard changes.
//!
//! Captures text and image content from the system clipboard. Text is stored
//! inline in the event; images are PNG-encoded and staged to a file. The
//! tracker emits exactly once per clipboard change — if the content doesn't
//! change between polls, no event is emitted.
//!
//! Change detection for text compares against previous content. For images,
//! the byte length of the raw RGBA buffer is compared first (different
//! dimensions = definite change), then a hash of the pixel data detects
//! same-dimension content changes.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use super::merge::{ClipboardContent, Event};

/// What kind of content was last seen on the clipboard.
#[derive(Debug)]
enum LastContent {
    /// No content observed yet (initial state).
    Empty,
    /// Last content was text.
    Text(String),
    /// Last content was an image (byte length + hash of pixel data).
    Image { byte_len: usize, hash: u64 },
}

/// Pure state machine for clipboard change detection.
///
/// Tracks the previous clipboard content and emits events only when the
/// content changes. Whitespace-only text is treated as empty.
pub(crate) struct ClipboardTracker {
    last: LastContent,
}

/// Result of checking the clipboard for changes.
#[derive(Debug)]
pub(crate) enum ClipboardUpdate {
    /// New content detected: emit an event with this content.
    Changed(ClipboardContent),
    /// No change since last check.
    Unchanged,
}

impl ClipboardTracker {
    /// Create a new tracker, seeded with the current clipboard state.
    ///
    /// Seeding captures the initial content without emitting an event —
    /// we only capture changes during the session.
    pub fn new_seeded(text: Option<&str>, image: Option<&ImageData>) -> Self {
        let last = match (text, image) {
            (Some(t), _) if !t.trim().is_empty() => LastContent::Text(t.to_string()),
            (_, Some(img)) => LastContent::Image {
                byte_len: img.bytes.len(),
                hash: hash_image_data(img),
            },
            _ => LastContent::Empty,
        };
        Self { last }
    }

    /// Check new clipboard content against the previous state.
    ///
    /// Text takes priority when both are available (can happen on some
    /// platforms with formatted text). Returns `Changed` with the new
    /// content on change, `Unchanged` otherwise.
    pub fn check(&mut self, text: Option<&str>, image: Option<&ImageData>) -> ClipboardUpdate {
        // Text takes priority when both are available.
        if let Some(t) = text {
            if t.trim().is_empty() {
                // Whitespace-only: treat as empty, no event.
                return ClipboardUpdate::Unchanged;
            }
            if matches!(&self.last, LastContent::Text(prev) if prev == t) {
                return ClipboardUpdate::Unchanged;
            }
            self.last = LastContent::Text(t.to_string());
            return ClipboardUpdate::Changed(ClipboardContent::Text {
                text: t.to_string(),
            });
        }

        if let Some(img) = image {
            let new_len = img.bytes.len();
            let new_hash = hash_image_data(img);
            if matches!(&self.last, LastContent::Image { byte_len, hash }
                if *byte_len == new_len && *hash == new_hash)
            {
                return ClipboardUpdate::Unchanged;
            }
            self.last = LastContent::Image {
                byte_len: new_len,
                hash: new_hash,
            };
            // Image path is filled in by the caller (spawn thread) after PNG encoding.
            return ClipboardUpdate::Changed(ClipboardContent::Image {
                path: String::new(),
            });
        }

        // Both unavailable: no event.
        ClipboardUpdate::Unchanged
    }
}

/// Minimal image data representation for change detection.
///
/// Mirrors the relevant fields from `arboard::ImageData` so that the
/// tracker's core logic can be tested without the clipboard crate.
pub(crate) struct ImageData {
    pub width: usize,
    pub height: usize,
    pub bytes: Vec<u8>,
}

/// Hash image pixel data for change detection.
///
/// Uses the standard library's `DefaultHasher` on the full byte slice.
/// This is fast enough for 500ms polling intervals.
fn hash_image_data(img: &ImageData) -> u64 {
    let mut hasher = DefaultHasher::new();
    img.bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests;
