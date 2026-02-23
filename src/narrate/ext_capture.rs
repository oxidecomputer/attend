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

/// Minimum dwell time before emitting an external selection (ms).
///
/// Only emit after the selected text has been stable for this long. Prevents
/// rapid re-selections (e.g. double-click expanding to word then sentence)
/// from generating multiple events.
const EXT_DWELL_MS: u64 = 300;

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
pub(crate) struct ExtDwellTracker {
    dwell_duration: Duration,
    /// The last emitted selection (for deduplication).
    prev_text: Option<String>,
    /// A selection waiting for the dwell timeout.
    pending: Option<(Instant, ExternalSnapshot)>,
}

impl ExtDwellTracker {
    /// Create a new tracker with the given dwell duration.
    pub fn new(dwell_duration: Duration) -> Self {
        Self {
            dwell_duration,
            prev_text: None,
            pending: None,
        }
    }

    /// Check whether a pending selection has dwelled long enough.
    ///
    /// Returns `Some(snapshot)` if the pending selection should be emitted now,
    /// clearing the pending state. Returns `None` otherwise.
    pub fn tick(&mut self, now: Instant) -> Option<ExternalSnapshot> {
        if let Some((changed_at, _)) = &self.pending
            && now.duration_since(*changed_at) >= self.dwell_duration
        {
            let (_, snapshot) = self.pending.take().unwrap();
            self.prev_text = snapshot.selected_text.clone();
            return Some(snapshot);
        }
        None
    }

    /// Process a new external snapshot.
    ///
    /// Returns `None` always — selections are deferred to [`tick`]. If the
    /// selected text changed, the pending snapshot is replaced. If it matches
    /// the previously emitted text, the pending is cleared (dedup).
    pub fn update(&mut self, snapshot: ExternalSnapshot, now: Instant) {
        let current_text = snapshot.selected_text.as_deref();

        // No text selected: clear pending, nothing to emit.
        if current_text.is_none() || current_text == Some("") {
            self.pending = None;
            return;
        }

        // Deduplicate: same text as the last emission, skip.
        if current_text == self.prev_text.as_deref() {
            self.pending = None;
            return;
        }

        // Check if this differs from the current pending.
        if let Some((_, ref pending_snap)) = self.pending
            && pending_snap.selected_text == snapshot.selected_text
        {
            // Same text still pending, let the dwell timer continue.
            return;
        }

        // New or changed selection: reset the dwell timer.
        self.pending = Some((now, snapshot));
    }
}

/// Spawn the external selection polling thread.
///
/// Returns the join handle, or `None` if the platform has no backend or
/// accessibility permission is not granted. The thread pushes
/// `ExternalSelection` events into `events` until `stop` is set.
pub(super) fn spawn(
    stop: Arc<AtomicBool>,
    events: Arc<Mutex<Vec<Event>>>,
    start: Instant,
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
        let mut tracker = ExtDwellTracker::new(Duration::from_millis(EXT_DWELL_MS));

        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(EXT_POLL_MS));

            let now = Instant::now();

            // Flush a dwelled selection if enough time has passed.
            if let Some(snapshot) = tracker.tick(now) {
                let offset_secs = start.elapsed().as_secs_f64();
                events.lock().unwrap().push(Event::ExternalSelection {
                    offset_secs,
                    app: snapshot.app,
                    window_title: snapshot.window_title,
                    text: snapshot.selected_text.unwrap_or_default(),
                });
            }

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

            tracker.update(snapshot, now);
        }

        // Final flush: emit any pending selection on shutdown.
        let now = Instant::now();
        // Force-flush by checking with a far-future time.
        if let Some(snapshot) = tracker.tick(now + Duration::from_secs(3600)) {
            let offset_secs = start.elapsed().as_secs_f64();
            events.lock().unwrap().push(Event::ExternalSelection {
                offset_secs,
                app: snapshot.app,
                window_title: snapshot.window_title,
                text: snapshot.selected_text.unwrap_or_default(),
            });
        }
    }))
}

#[cfg(test)]
mod tests;
