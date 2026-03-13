//! Background polling of editor selections.
//!
//! Spawns a thread that polls an [`EditorStateSource`] every 100ms and emits
//! [`Event::EditorSnapshot`] whenever the file/selection set changes.
//!
//! Cursor dwell logic is encapsulated in [`DwellTracker`]: cursor-only
//! snapshots are deferred until the cursor rests for a configurable
//! duration, while real selections emit immediately.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};
use chrono::{DateTime, TimeDelta, Utc};

use super::merge::Event;
use crate::clock::Clock;
use crate::state::{self, EditorState};
use crate::view;

/// Source of editor state snapshots.
///
/// Abstracts the platform-specific editor query so tests can substitute
/// a stub that returns scripted file lists and cursor positions.
/// The production implementation calls [`EditorState::current`].
pub trait EditorStateSource: Send {
    /// Query the current editor state.
    fn current(
        &self,
        cwd: Option<&Utf8Path>,
        include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Option<EditorState>>;
}

/// Production implementation: queries real editor(s) via platform APIs.
pub(crate) struct RealEditorSource;

impl EditorStateSource for RealEditorSource {
    fn current(
        &self,
        cwd: Option<&Utf8Path>,
        include_dirs: &[Utf8PathBuf],
    ) -> anyhow::Result<Option<EditorState>> {
        EditorState::current(cwd, include_dirs)
    }
}

/// How often to poll for editor selection changes (ms).
const EDITOR_POLL_MS: u64 = 100;

/// Minimum dwell time before emitting a cursor-only snapshot (ms).
///
/// Rapid file scanning generates many low-value cursor positions. We only
/// emit a cursor-only snapshot once the cursor has rested at a position for
/// at least this long. Snapshots with real selections (highlights) are
/// emitted immediately since they indicate intentional pointing.
const CURSOR_DWELL_MS: u64 = 500;

/// Result of processing an editor state update through the dwell tracker.
#[derive(Debug)]
enum EditorUpdate {
    /// Changed state with real selections: emit immediately.
    Emit(Vec<state::FileEntry>),
    /// Unchanged state with real selections: extend last event's `last_seen`.
    Extend,
    /// Cursor-only or truly no change: pending/skip.
    None,
}

/// Pure state machine for cursor dwell filtering.
///
/// Tracks previous editor state and a pending cursor-only snapshot. The
/// caller drives the tracker via [`tick`] (time-based flush) and [`update`]
/// (new editor state). When either method returns `Some`, the caller should
/// emit that snapshot.
struct DwellTracker {
    dwell_duration: TimeDelta,
    /// Last emitted-or-stored file state, used for deduplication.
    prev_files: Option<Vec<state::FileEntry>>,
    /// A cursor-only snapshot waiting for the dwell timeout.
    pending_cursor: Option<(DateTime<Utc>, Vec<state::FileEntry>)>,
}

impl DwellTracker {
    /// Create a new tracker with the given dwell duration.
    fn new(dwell_duration: TimeDelta) -> Self {
        Self {
            dwell_duration,
            prev_files: None,
            pending_cursor: None,
        }
    }

    /// Check whether a pending cursor-only snapshot has dwelled long enough.
    ///
    /// Returns `Some(files)` if the pending snapshot should be emitted now,
    /// clearing the pending state. Returns `None` otherwise.
    fn tick(&mut self, now: DateTime<Utc>) -> Option<Vec<state::FileEntry>> {
        if let Some((changed_at, _)) = &self.pending_cursor
            && (now - *changed_at) >= self.dwell_duration
        {
            return self.pending_cursor.take().map(|(_, files)| files);
        }
        None
    }

    /// Process a new editor state snapshot.
    ///
    /// Returns `Emit(files)` if the snapshot should be emitted immediately
    /// (i.e. it contains real selections and is different from previous).
    /// Returns `Extend` if unchanged but has real selections (for `last_seen`).
    /// Cursor-only snapshots are stored as pending and will be emitted later
    /// via [`tick`]. Returns `None` for cursor-only or truly unchanged cursors.
    fn update(&mut self, files: Vec<state::FileEntry>, now: DateTime<Utc>) -> EditorUpdate {
        // Check if unchanged from last state.
        if self.prev_files.as_ref() == Some(&files) {
            // If the unchanged state has real selections, signal Extend
            // so the caller can update last_seen. Cursor-only duplicates
            // are just skipped.
            let has_real_selection = files
                .iter()
                .any(|f| f.selections.iter().any(|s| !s.is_cursor_like()));
            if has_real_selection {
                return EditorUpdate::Extend;
            }
            return EditorUpdate::None;
        }

        self.prev_files = Some(files.clone());

        let cursor_only = files
            .iter()
            .all(|f| f.selections.iter().all(|s| s.is_cursor_like()));

        if cursor_only {
            // Defer emission until the cursor dwells at this position.
            self.pending_cursor = Some((now, files));
            EditorUpdate::None
        } else {
            // Real selections are always intentional: emit immediately
            // and discard any pending cursor-only snapshot.
            self.pending_cursor = None;
            EditorUpdate::Emit(files)
        }
    }
}

/// Spawn the editor polling thread.
///
/// Returns the join handle. The thread pushes `EditorSnapshot` events into
/// `events` until stopped via `control`. It also publishes the current set
/// of open file paths into `open_paths` for the diff capture thread to read.
pub(super) fn spawn(
    source: Box<dyn EditorStateSource>,
    clock: Arc<dyn Clock>,
    control: Arc<super::capture::CaptureControl>,
    cwd: Option<Utf8PathBuf>,
    events: Arc<Mutex<Vec<Event>>>,
    open_paths: Arc<Mutex<Vec<Utf8PathBuf>>>,
) -> std::thread::JoinHandle<()> {
    crate::clock::spawn_clock_thread("editor", &*clock, move |clock| {
        let mut lang_cache = view::LanguageCache::new();
        let mut tracker = DwellTracker::new(TimeDelta::milliseconds(CURSOR_DWELL_MS as i64));

        loop {
            if control.wait_while_paused(&*clock) {
                break;
            }

            clock.sleep(Duration::from_millis(EDITOR_POLL_MS));

            let now = clock.now();

            // Flush a dwelled cursor-only snapshot if enough time has passed.
            // Tick before update: a pending cursor that has dwelled long enough
            // is emitted using a fresh timestamp, then the new poll proceeds.
            if let Some(files) = tracker.tick(now) {
                let snapshot = make_editor_snapshot(files, clock.now(), &mut lang_cache);
                let Ok(mut guard) = events.lock() else {
                    tracing::error!("event mutex poisoned: editor capture thread exiting");
                    break;
                };
                guard.push(snapshot);
            }

            let state = match source.current(cwd.as_deref(), &[]) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            // Publish open file paths for the diff capture thread.
            {
                let Ok(mut guard) = open_paths.lock() else {
                    tracing::error!("open_paths mutex poisoned: editor capture thread exiting");
                    break;
                };
                *guard = state.files.iter().map(|f| f.path.clone()).collect();
            }

            match tracker.update(state.files, now) {
                EditorUpdate::Emit(files) => {
                    let snapshot = make_editor_snapshot(files, clock.now(), &mut lang_cache);
                    let Ok(mut guard) = events.lock() else {
                        tracing::error!("event mutex poisoned: editor capture thread exiting");
                        break;
                    };
                    guard.push(snapshot);
                }
                EditorUpdate::Extend => {
                    let now_utc = clock.now();
                    let Ok(mut guard) = events.lock() else {
                        tracing::error!("event mutex poisoned: editor capture thread exiting");
                        break;
                    };
                    if let Some(Event::EditorSnapshot { last_seen, .. }) = guard.last_mut() {
                        *last_seen = now_utc;
                    }
                }
                EditorUpdate::None => {}
            }
        }
    })
}

/// Build an `EditorSnapshot` event from a set of file entries.
///
/// The caller must obtain the `timestamp` via `clock.now()` *after* calling
/// `tick()` / `update()` so that the snapshot timestamp reflects the moment
/// we decided to emit, not the moment we polled.
fn make_editor_snapshot(
    files: Vec<state::FileEntry>,
    timestamp: DateTime<Utc>,
    lang_cache: &mut view::LanguageCache,
) -> Event {
    let regions = capture_snapshot_regions(&files, None, lang_cache);
    Event::EditorSnapshot {
        timestamp,
        last_seen: timestamp,
        files,
        regions,
    }
}

/// Capture file regions for an editor snapshot.
fn capture_snapshot_regions(
    files: &[state::FileEntry],
    cwd: Option<&Utf8Path>,
    lang_cache: &mut view::LanguageCache,
) -> Vec<view::CapturedRegion> {
    let extent = view::Extent::Lines {
        before: 1,
        after: 1,
    };
    view::capture_regions(files, cwd, extent, lang_cache).unwrap_or_default()
}

#[cfg(test)]
mod tests;
