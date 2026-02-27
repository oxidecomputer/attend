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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::merge::{ClipboardContent, Event};
use crate::clock::Clock;

/// Source of clipboard content.
///
/// Abstracts the platform clipboard so tests can substitute a stub that
/// returns scripted content without touching the real system clipboard.
/// The production implementation wraps [`arboard::Clipboard`].
pub trait ClipboardSource: Send {
    /// Read the current clipboard text, if any.
    fn get_text(&mut self) -> Option<String>;
    /// Read the current clipboard image, if any.
    fn get_image(&mut self) -> Option<ImageData>;
}

/// Production implementation: reads from the system clipboard via arboard.
pub(crate) struct ArboardClipboardSource {
    clipboard: arboard::Clipboard,
}

impl ArboardClipboardSource {
    /// Create a new source, or `None` if the clipboard is unavailable.
    pub fn new() -> Option<Self> {
        match arboard::Clipboard::new() {
            Ok(clipboard) => Some(Self { clipboard }),
            Err(e) => {
                tracing::warn!("Clipboard capture unavailable: {e}");
                None
            }
        }
    }
}

impl ClipboardSource for ArboardClipboardSource {
    fn get_text(&mut self) -> Option<String> {
        self.clipboard.get_text().ok()
    }

    fn get_image(&mut self) -> Option<ImageData> {
        self.clipboard.get_image().ok().map(|img| ImageData {
            width: img.width,
            height: img.height,
            bytes: img.bytes.into_owned(),
        })
    }
}

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

/// How often to poll the clipboard for changes (ms).
const CLIPBOARD_POLL_MS: u64 = 500;

/// Spawn the clipboard polling thread.
///
/// Returns the join handle. The thread pushes `ClipboardSelection`
/// events into `events` until `stop` is set.
///
/// Unlike other capture threads, clipboard polling has no paused state.
/// Instead, the thread is killed on pause and a fresh one is spawned on
/// resume. This avoids a race where clipboard changes made while paused
/// (e.g. yank copying rendered narration) would be captured as events
/// in the next recording period.
pub(super) fn spawn(
    mut source: Box<dyn ClipboardSource>,
    clock: Arc<dyn Clock>,
    stop: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Event>>>,
    staging_dir: camino::Utf8PathBuf,
) -> Option<std::thread::JoinHandle<()>> {
    // Seed with current clipboard content (no event emitted).
    let seed_text = source.get_text();
    let seed_image = source.get_image();
    let mut tracker = ClipboardTracker::new_seeded(seed_text.as_deref(), seed_image.as_ref());

    Some(std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            clock.sleep(Duration::from_millis(CLIPBOARD_POLL_MS));

            // Try text first, then image. First success wins.
            let text = source.get_text();
            let image_data = source.get_image();

            match tracker.check(text.as_deref(), image_data.as_ref()) {
                ClipboardUpdate::Changed(ClipboardContent::Text { text }) => {
                    let timestamp = clock.now();
                    events.lock().unwrap().push(Event::ClipboardSelection {
                        timestamp,
                        content: ClipboardContent::Text { text },
                    });
                }
                ClipboardUpdate::Changed(ClipboardContent::Image { .. }) => {
                    // Encode the image to PNG and stage it.
                    let Some(ref img) = image_data else {
                        continue;
                    };
                    let Some(path) = stage_image_png(img, &staging_dir) else {
                        continue;
                    };
                    let timestamp = clock.now();
                    events.lock().unwrap().push(Event::ClipboardSelection {
                        timestamp,
                        content: ClipboardContent::Image { path },
                    });
                }
                ClipboardUpdate::Unchanged => {}
            }
        }
    }))
}

/// Encode image data to PNG and write to the clipboard staging directory.
///
/// Returns the absolute path to the staged file, or `None` on failure.
fn stage_image_png(img: &ImageData, staging_dir: &camino::Utf8Path) -> Option<String> {
    use image::{ImageBuffer, Rgba};

    let buf: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(img.width as u32, img.height as u32, &img.bytes[..])?;

    if let Err(e) = std::fs::create_dir_all(staging_dir) {
        tracing::warn!("Cannot create clipboard staging dir: {e}");
        return None;
    }

    let ts = crate::util::utc_now_nanos().replace(':', "-");
    let id = uuid::Uuid::new_v4();
    let path = staging_dir.join(format!("{ts}-{id}.png"));

    if let Err(e) = buf.save(path.as_str()) {
        tracing::warn!("Cannot encode clipboard image to PNG: {e}");
        return None;
    }

    Some(path.to_string())
}

#[cfg(test)]
mod tests;
