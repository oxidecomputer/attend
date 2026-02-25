//! Background polling of text selected in external applications.
//!
//! Captures selected text from the focused application via platform-specific
//! accessibility APIs (macOS AX, future: Linux AT-SPI). The capture loop is
//! OS-agnostic: it polls an [`ExternalSource`] trait for the current selection
//! state and emits [`Event::ExternalSelection`] when the selection stabilizes.
//!
//! Cursor dwell logic is encapsulated in [`ExtDwellTracker`]: selections are
//! deferred until the text is stable for a configurable duration, preventing
//! rapid intermediate selections from flooding the event stream.

#[cfg(target_os = "macos")]
mod macos;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::merge::Event;

/// How often to poll for external selection changes (ms).
const EXT_POLL_MS: u64 = 200;

/// Sleep interval when paused (ms).
const PAUSED_POLL_MS: u64 = 500;

/// Minimum dwell time before emitting an external selection (ms).
/// A snapshot of the currently selected text in the focused application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExternalSnapshot {
    /// Application name (e.g. "Safari", "iTerm2").
    pub app: String,
    /// Window title (e.g. page title, terminal tab name).
    pub window_title: String,
    /// The selected text, if any.
    pub selected_text: Option<String>,
}

/// Platform-specific backend for querying external application state.
pub trait ExternalSource: Send {
    /// Check whether the platform's accessibility permission is granted.
    /// Returns `false` if queries will fail due to missing permissions.
    fn is_available(&self) -> bool;

    /// Query the current state of the focused application.
    /// Returns `None` if the query fails or no application is focused.
    fn query(&self) -> Option<ExternalSnapshot>;
}

/// Construct the platform-appropriate ExternalSource, or None if the
/// current platform has no backend.
pub fn platform_source() -> Option<Box<dyn ExternalSource>> {
    #[cfg(target_os = "macos")]
    {
        Some(Box::new(macos::MacOsSource::new()))
    }

    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Pure state machine for external selection dwell filtering.
///
/// Tracks the previous selection state and a pending selection. The caller
/// drives the tracker via [`tick`] (time-based flush) and [`update`] (new
/// snapshot). When either method returns `Some`, the caller should emit
/// an `ExternalSelection` event.
/// Tracks the last emitted external selection for deduplication.
///
/// Unlike the editor cursor dwell tracker, external selections are emitted
/// immediately on change (no dwell timer). Deduplication of progressive
/// selections (drag-to-select) is handled downstream by the merge pipeline.
pub(crate) struct ExtDwellTracker {
    /// The last emitted selection text (for exact-match dedup).
    prev_text: Option<String>,
}

impl ExtDwellTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self { prev_text: None }
    }

    /// Process a new external snapshot.
    ///
    /// Returns `Some(snapshot)` immediately if the selected text changed.
    /// Returns `None` if unchanged (exact-match dedup) or empty.
    pub fn update(
        &mut self,
        snapshot: ExternalSnapshot,
        _now: Instant,
    ) -> Option<ExternalSnapshot> {
        let current_text = snapshot.selected_text.as_deref();

        // No text selected: nothing to emit.
        if current_text.is_none() || current_text == Some("") {
            return None;
        }

        // Deduplicate: same text as the last emission, skip.
        if current_text == self.prev_text.as_deref() {
            return None;
        }

        self.prev_text = snapshot.selected_text.clone();
        Some(snapshot)
    }
}

/// Spawn the external selection polling thread.
///
/// Returns the join handle, or `None` if the platform has no backend or
/// accessibility permission is not granted. The thread pushes
/// `ExternalSelection` events into `events` until `stop` is set.
pub(super) fn spawn(
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Event>>>,
    ignore_apps: Vec<String>,
) -> Option<thread::JoinHandle<()>> {
    let source = platform_source()?;

    if !source.is_available() {
        eprintln!(
            "External text capture requires Accessibility permission for your terminal. \
             Grant it in System Settings > Privacy & Security > Accessibility."
        );
        return None;
    }

    Some(thread::spawn(move || {
        let mut tracker = ExtDwellTracker::new();

        while !stop.load(Ordering::Relaxed) {
            if paused.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(PAUSED_POLL_MS));
                continue;
            }

            thread::sleep(Duration::from_millis(EXT_POLL_MS));

            let now = Instant::now();

            // Query the platform backend.
            let Some(snapshot) = source.query() else {
                continue;
            };

            // Skip apps in the ignore list (case-insensitive).
            if ignore_apps
                .iter()
                .any(|ignored| ignored.eq_ignore_ascii_case(&snapshot.app))
            {
                continue;
            }

            // Emit immediately on change with UTC timestamp.
            if let Some(snapshot) = tracker.update(snapshot, now) {
                let timestamp = chrono::Utc::now();
                events.lock().unwrap().push(Event::ExternalSelection {
                    timestamp,
                    app: snapshot.app,
                    window_title: snapshot.window_title,
                    text: snapshot.selected_text.unwrap_or_default(),
                });
            }
        }
    }))
}

#[cfg(test)]
mod tests;
