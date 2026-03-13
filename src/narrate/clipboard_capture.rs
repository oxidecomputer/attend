//! Background polling of system clipboard changes.
//!
//! Captures text and image content from the system clipboard. Text is stored
//! inline in the event; images are PNG-encoded and staged to a file. The
//! tracker emits exactly once per clipboard change — if the content doesn't
//! change between polls, no event is emitted.
//!
//! Change detection for text compares against previous content. For images,
//! a blake3 hash of the raw RGBA pixel data detects content changes.

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
    /// Last content was an image (blake3 hash of pixel data).
    Image(blake3::Hash),
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
            (_, Some(img)) => LastContent::Image(hash_image_data(img)),
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
            let new_hash = hash_image_data(img);
            if matches!(&self.last, LastContent::Image(prev) if *prev == new_hash) {
                return ClipboardUpdate::Unchanged;
            }
            self.last = LastContent::Image(new_hash);
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
/// Uses blake3 on the full byte slice. blake3 is faster than SipHash
/// for large buffers and produces a 256-bit hash, eliminating any
/// realistic chance of collision.
fn hash_image_data(img: &ImageData) -> blake3::Hash {
    blake3::hash(&img.bytes)
}

/// How often to poll the clipboard for changes (ms).
const CLIPBOARD_POLL_MS: u64 = 500;

/// Spawn the clipboard polling thread.
///
/// Returns the join handle. The thread pushes `ClipboardSelection`
/// events into `events` until stopped via `control`.
///
/// On resume, the `clipboard_reseed` flag in `CaptureControl` tells
/// the thread to re-seed its tracker from the current clipboard, so
/// changes made while paused (e.g. yank copying rendered narration)
/// are treated as baseline, not as new events.
pub(super) fn spawn(
    mut source: Box<dyn ClipboardSource>,
    clock: Arc<dyn Clock>,
    control: Arc<super::capture::CaptureControl>,
    events: Arc<Mutex<Vec<Event>>>,
    staging_dir: camino::Utf8PathBuf,
) -> Option<std::thread::JoinHandle<()>> {
    // Seed with current clipboard content (no event emitted).
    let seed_text = source.get_text();
    let seed_image = source.get_image();
    let mut tracker = ClipboardTracker::new_seeded(seed_text.as_deref(), seed_image.as_ref());

    Some(crate::clock::spawn_clock_thread(
        "clipboard",
        &*clock,
        move |clock| {
            loop {
                if control.wait_while_paused(&*clock) {
                    break;
                }

                // Re-seed after resume: treat current clipboard as baseline
                // so changes during pause aren't captured.
                if control.take_clipboard_reseed() {
                    let text = source.get_text();
                    let image = source.get_image();
                    tracker = ClipboardTracker::new_seeded(text.as_deref(), image.as_ref());
                }

                clock.sleep(Duration::from_millis(CLIPBOARD_POLL_MS));

                // Try text first, then image. First success wins.
                let text = source.get_text();
                let image_data = source.get_image();

                match tracker.check(text.as_deref(), image_data.as_ref()) {
                    ClipboardUpdate::Changed(ClipboardContent::Text { text }) => {
                        let timestamp = clock.now();
                        let Ok(mut guard) = events.lock() else {
                            tracing::error!(
                                "event mutex poisoned: clipboard capture thread exiting"
                            );
                            break;
                        };
                        guard.push(Event::ClipboardSelection {
                            timestamp,
                            content: ClipboardContent::Text { text },
                        });
                    }
                    ClipboardUpdate::Changed(ClipboardContent::Image { .. }) => {
                        // Encode the image to PNG and stage it.
                        let Some(ref img) = image_data else {
                            continue;
                        };
                        let Some(path) = stage_image_png(img, &staging_dir, clock.now()) else {
                            continue;
                        };
                        let timestamp = clock.now();
                        let Ok(mut guard) = events.lock() else {
                            tracing::error!(
                                "event mutex poisoned: clipboard capture thread exiting"
                            );
                            break;
                        };
                        guard.push(Event::ClipboardSelection {
                            timestamp,
                            content: ClipboardContent::Image { path },
                        });
                    }
                    ClipboardUpdate::Unchanged => {}
                }
            }
        },
    ))
}

/// Encode image data to PNG and write to the clipboard staging directory.
///
/// Returns the absolute path to the staged file, or `None` on failure.
fn stage_image_png(
    img: &ImageData,
    staging_dir: &camino::Utf8Path,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<String> {
    use image::{ImageBuffer, Rgba};

    let buf: ImageBuffer<Rgba<u8>, _> =
        ImageBuffer::from_raw(img.width as u32, img.height as u32, &img.bytes[..])?;

    if let Err(e) = std::fs::create_dir_all(staging_dir) {
        tracing::warn!("Cannot create clipboard staging dir: {e}");
        return None;
    }

    let ts = crate::util::format_utc_nanos(now).replace(':', "-");
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
