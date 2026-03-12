//! Background capture of editor state snapshots, file diffs, and external selections.
//!
//! Coordinates up to four capture threads with shared state:
//! - [`editor_capture`]: polls editor selections, emits `EditorSnapshot`,
//!   and publishes the current set of open file paths.
//! - [`diff_capture`]: reads the shared file list (instead of querying the
//!   editor independently) and watches for content changes via mtime.
//! - [`ext_capture`]: polls the platform accessibility API for selected text
//!   in external applications (e.g. iTerm2, Safari).
//! - [`clipboard_capture`]: polls the system clipboard for text/image changes.
//!
//! Pause/resume is coordinated by [`CaptureControl`]: paused threads block
//! on a condvar (zero CPU). The [`SyncClock::park()`] guard brackets each
//! condvar wait, registering the thread as settled so
//! `advance_and_settle()` completes without waiting for the paused thread.
//! Resume notifies the condvar for instant wakeup.
//!
//! On resume, the clipboard thread re-seeds its tracker from the current
//! clipboard content, so changes made during pause (e.g. yank copying
//! rendered narration) aren't captured as events.
//!
//! All platform dependencies are behind traits ([`EditorStateSource`],
//! [`ExternalSource`], [`ClipboardSource`]) so tests can substitute stubs.
//! The [`CaptureConfig`] struct bundles these for dependency injection.

use std::sync::{Arc, Condvar, Mutex};
use std::thread;

use camino::Utf8PathBuf;

use crate::clock::{Clock, SyncClock};

use super::clipboard_capture::ClipboardSource;
use super::editor_capture::EditorStateSource;
use super::ext_capture::ExternalSource;
use super::merge::Event;

/// Bundled capture source factories for dependency injection.
///
/// Production code uses [`CaptureConfig::production`]; test mode substitutes
/// stubs that return scripted state.
pub(crate) struct CaptureConfig {
    /// Injectable clock for timestamps and sleep.
    pub clock: Arc<dyn Clock>,
    /// Editor state query source.
    pub editor_source: Box<dyn EditorStateSource>,
    /// External selection source, or `None` if unavailable.
    pub ext_source: Option<Box<dyn ExternalSource>>,
    /// Clipboard source, or `None` if unavailable. Used once at startup.
    pub clipboard_source: Option<Box<dyn ClipboardSource>>,
}

impl CaptureConfig {
    /// Create a config using real platform sources.
    pub fn production(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            editor_source: Box::new(super::editor_capture::RealEditorSource),
            ext_source: super::ext_capture::platform_source(),
            clipboard_source: super::clipboard_capture::ArboardClipboardSource::new()
                .map(|s| Box::new(s) as Box<dyn ClipboardSource>),
        }
    }

    /// Create a config using test stubs backed by the inject router's
    /// shared state (created during `test_mode::init()`).
    ///
    /// Returns the config, plus a `StubTranscriber` for the daemon to use.
    pub fn test_mode(
        clock: Arc<dyn Clock>,
    ) -> (Self, crate::narrate::transcribe::stub::StubTranscriber) {
        use crate::test_mode::stubs::*;

        // The inject router was registered during init(). Pull shared
        // state from it to construct stubs that read the same Arcs.
        let router = crate::test_mode::router();
        let editor_state = Arc::clone(&router.editor_state);
        let ext_snapshot = Arc::clone(&router.ext_snapshot);
        let clipboard_text = Arc::clone(&router.clipboard_text);

        let config = Self {
            clock,
            editor_source: Box::new(StubEditorSource::new(editor_state)),
            ext_source: Some(Box::new(StubExternalSource::new(ext_snapshot))),
            clipboard_source: Some(
                Box::new(StubClipboardSource::new(clipboard_text)) as Box<dyn ClipboardSource>
            ),
        };

        let stub_transcriber = crate::test_mode::take_stub_transcriber();
        (config, stub_transcriber)
    }
}

/// Shared pause/stop/reseed state for all capture threads.
///
/// Paused threads block on the condvar (zero CPU). The
/// [`SyncClock::park()`] guard brackets each condvar wait, registering
/// the thread as settled so `advance_and_settle()` completes without
/// waiting for the paused thread. Resume notifies the condvar for
/// instant wakeup.
pub(crate) struct CaptureControl {
    state: Mutex<ControlState>,
    condvar: Condvar,
}

struct ControlState {
    paused: bool,
    stopped: bool,
    /// Set on resume: tells the clipboard thread to re-seed its tracker.
    clipboard_reseed: bool,
}

impl CaptureControl {
    /// Create a new control in the running (not paused, not stopped) state.
    fn new() -> Self {
        Self {
            state: Mutex::new(ControlState {
                paused: false,
                stopped: false,
                clipboard_reseed: false,
            }),
            condvar: Condvar::new(),
        }
    }

    /// Block while paused, returning `true` if stopped.
    ///
    /// Each condvar wait is bracketed by a [`Clock::park()`] guard so the
    /// settlement protocol sees the thread as settled while it blocks.
    pub fn wait_while_paused(&self, clock: &dyn SyncClock) -> bool {
        let mut state = self.state.lock().unwrap();
        while state.paused && !state.stopped {
            // Park guard brackets the condvar wait: settled += 1 on
            // creation, expected += 1 on drop after wake.
            let _guard = clock.park();
            #[allow(clippy::disallowed_methods)]
            {
                state = self.condvar.wait(state).unwrap();
            }
        }
        state.stopped
    }

    /// Atomically read and clear the clipboard reseed flag.
    pub fn take_clipboard_reseed(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        let val = state.clipboard_reseed;
        state.clipboard_reseed = false;
        val
    }

    /// Pause all capture threads.
    ///
    /// Running threads will see the flag on their next iteration and
    /// block on the condvar.
    fn pause(&self) {
        self.state.lock().unwrap().paused = true;
    }

    /// Resume all capture threads.
    ///
    /// Sets `clipboard_reseed` so the clipboard thread treats the current
    /// clipboard content as baseline. Notifies the condvar so paused
    /// threads wake immediately.
    fn resume(&self) {
        let mut state = self.state.lock().unwrap();
        state.clipboard_reseed = true;
        state.paused = false;
        drop(state);
        self.condvar.notify_all();
    }

    /// Stop all capture threads.
    ///
    /// Notifies the condvar so paused threads wake and exit.
    fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        state.stopped = true;
        drop(state);
        self.condvar.notify_all();
    }
}

/// Handle for the background editor/diff/ext/clipboard polling threads.
pub(crate) struct CaptureHandle {
    control: Arc<CaptureControl>,
    editor_events: Arc<Mutex<Vec<Event>>>,
    diff_events: Arc<Mutex<Vec<Event>>>,
    ext_events: Arc<Mutex<Vec<Event>>>,
    clipboard_events: Arc<Mutex<Vec<Event>>>,
    editor_thread: Option<thread::JoinHandle<()>>,
    diff_thread: Option<thread::JoinHandle<()>>,
    ext_thread: Option<thread::JoinHandle<()>>,
    clipboard_thread: Option<thread::JoinHandle<()>>,
}

impl CaptureHandle {
    /// Pause all capture threads.
    ///
    /// Running threads will see the flag on their next iteration and
    /// block on the condvar (zero CPU, settled via park guard).
    pub fn pause(&mut self) {
        self.control.pause();
    }

    /// Resume all capture threads.
    ///
    /// Wakes paused threads immediately via condvar notify. Sets the
    /// clipboard reseed flag so changes during pause aren't captured.
    pub fn resume(&mut self) {
        self.control.resume();
    }

    /// Drain accumulated events without stopping threads.
    pub fn drain(&self) -> (Vec<Event>, Vec<Event>, Vec<Event>, Vec<Event>) {
        let editor = std::mem::take(&mut *self.editor_events.lock().unwrap());
        let diff = std::mem::take(&mut *self.diff_events.lock().unwrap());
        let ext = std::mem::take(&mut *self.ext_events.lock().unwrap());
        let clipboard = std::mem::take(&mut *self.clipboard_events.lock().unwrap());
        (editor, diff, ext, clipboard)
    }

    /// Signal stop and collect remaining results.
    pub fn collect(mut self) -> (Vec<Event>, Vec<Event>, Vec<Event>, Vec<Event>) {
        self.control.stop();

        // Intentionally ignored: thread panics are non-recoverable here.
        if let Some(h) = self.editor_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.diff_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.ext_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.clipboard_thread.take() {
            let _ = h.join();
        }

        self.drain()
    }
}

/// Start background threads for editor polling, file diff tracking, and
/// external selection capture.
///
/// All capture threads use `Utc::now()` for event timestamps, so there is
/// no shared time epoch to maintain. The recording daemon computes word
/// timestamps from `period_start_utc + word.start_secs`, which is also UTC.
///
/// Pass `None` for `cwd` to keep paths absolute (filtering deferred to receive).
pub(crate) fn start(
    config: CaptureConfig,
    cwd: Option<Utf8PathBuf>,
    ext_ignore_apps: Vec<String>,
    clipboard_capture: bool,
    clipboard_staging_dir: camino::Utf8PathBuf,
) -> anyhow::Result<CaptureHandle> {
    let control = Arc::new(CaptureControl::new());

    let editor_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let diff_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let ext_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let clipboard_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));

    // Shared file path list: written by editor_capture, read by diff_capture.
    let open_paths: Arc<Mutex<Vec<Utf8PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let editor_thread = super::editor_capture::spawn(
        config.editor_source,
        Arc::clone(&config.clock),
        Arc::clone(&control),
        cwd,
        Arc::clone(&editor_events),
        Arc::clone(&open_paths),
    );

    let diff_thread = super::diff_capture::spawn(
        Arc::clone(&config.clock),
        Arc::clone(&control),
        Arc::clone(&open_paths),
        Arc::clone(&diff_events),
    );

    let ext_thread = if let Some(ext_source) = config.ext_source {
        super::ext_capture::spawn(
            ext_source,
            Arc::clone(&config.clock),
            Arc::clone(&control),
            Arc::clone(&ext_events),
            ext_ignore_apps,
        )
    } else {
        None
    };

    let clipboard_thread = if clipboard_capture {
        if let Some(source) = config.clipboard_source {
            super::clipboard_capture::spawn(
                source,
                Arc::clone(&config.clock),
                Arc::clone(&control),
                Arc::clone(&clipboard_events),
                clipboard_staging_dir.clone(),
            )
        } else {
            None
        }
    } else {
        None
    };

    Ok(CaptureHandle {
        control,
        editor_events,
        diff_events,
        ext_events,
        clipboard_events,
        editor_thread: Some(editor_thread),
        diff_thread: Some(diff_thread),
        ext_thread,
        clipboard_thread,
    })
}

#[cfg(test)]
mod tests;
