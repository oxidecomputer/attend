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
use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::clock::Clock;

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

/// Result of processing an external snapshot through the dwell tracker.
#[derive(Debug)]
pub(crate) enum ExtUpdate {
    /// New selection text: push a new event.
    New(ExternalSnapshot),
    /// Same text as last emission: extend last event's `last_seen`.
    Extend,
    /// No text selected or nothing to report.
    None,
}

impl ExtDwellTracker {
    /// Create a new tracker.
    pub fn new() -> Self {
        Self { prev_text: None }
    }

    /// Process a new external snapshot.
    ///
    /// Returns `New(snapshot)` immediately if the selected text changed.
    /// Returns `Extend` if unchanged (same text, for `last_seen` update).
    /// Returns `None` if no text is selected.
    pub fn update(&mut self, snapshot: ExternalSnapshot, _now: DateTime<Utc>) -> ExtUpdate {
        let current_text = snapshot.selected_text.as_deref();

        // No text selected: nothing to emit.
        if current_text.is_none() || current_text == Some("") {
            return ExtUpdate::None;
        }

        // Same text as the last emission: extend last_seen.
        if current_text == self.prev_text.as_deref() {
            return ExtUpdate::Extend;
        }

        self.prev_text = snapshot.selected_text.clone();
        ExtUpdate::New(snapshot)
    }
}

/// Spawn the external selection polling thread.
///
/// Returns the join handle, or `None` if accessibility permission is not
/// granted. The thread pushes `ExternalSelection` events into `events`
/// until `stop` is set.
pub(super) fn spawn(
    source: Box<dyn ExternalSource>,
    clock: Arc<dyn Clock>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Event>>>,
    ignore_apps: Vec<String>,
) -> Option<thread::JoinHandle<()>> {
    if !source.is_available() {
        eprintln!(
            "External text capture requires Accessibility permission for your terminal. \
             Grant it in System Settings > Privacy & Security > Accessibility."
        );
        return None;
    }

    Some(crate::clock::spawn_clock_thread(
        "ext",
        &*clock,
        move |clock| {
            let mut tracker = ExtDwellTracker::new();

            while !stop.load(Ordering::Relaxed) {
                if paused.load(Ordering::Relaxed) {
                    clock.sleep(Duration::from_millis(PAUSED_POLL_MS));
                    continue;
                }

                clock.sleep(Duration::from_millis(EXT_POLL_MS));

                let now = clock.now();

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

                // Emit or extend based on tracker result.
                match tracker.update(snapshot, now) {
                    ExtUpdate::New(snapshot) => {
                        let timestamp = clock.now();
                        events.lock().unwrap().push(Event::ExternalSelection {
                            timestamp,
                            last_seen: timestamp,
                            app: snapshot.app,
                            window_title: snapshot.window_title,
                            text: snapshot.selected_text.unwrap_or_default(),
                        });
                    }
                    ExtUpdate::Extend => {
                        let now_utc = clock.now();
                        let mut guard = events.lock().unwrap();
                        if let Some(Event::ExternalSelection { last_seen, .. }) = guard.last_mut() {
                            *last_seen = now_utc;
                        }
                    }
                    ExtUpdate::None => {}
                }
            }
        },
    ))
}

#[cfg(test)]
mod tests;
