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
//! All threads (including clipboard) stay alive during pause, sleeping
//! with a slow poll interval. This is required so `MockClock::advance_and_settle`
//! can track all threads: a thread that exits its sleep loop without re-entering
//! breaks settlement tracking. On resume, the clipboard thread re-seeds its
//! tracker from the current clipboard content, so changes made during pause
//! (e.g. yank copying rendered narration) aren't captured as events.
//!
//! All platform dependencies are behind traits ([`EditorStateSource`],
//! [`ExternalSource`], [`ClipboardSource`]) so tests can substitute stubs.
//! The [`CaptureConfig`] struct bundles these for dependency injection.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use camino::Utf8PathBuf;

use crate::clock::Clock;

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

/// Handle for the background editor/diff/ext/clipboard polling threads.
pub(crate) struct CaptureHandle {
    stop_flag: Arc<AtomicBool>,
    paused_flag: Arc<AtomicBool>,
    /// Set on resume: tells the clipboard thread to re-seed its tracker
    /// from the current clipboard content, so changes during pause aren't
    /// captured as events in the next recording period.
    clipboard_reseed: Arc<AtomicBool>,
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
    /// All threads (editor, diff, ext, clipboard) enter a sleep loop
    /// via the shared paused flag. No threads are stopped — they stay
    /// alive so `MockClock::advance_and_settle` can track them.
    pub fn pause(&mut self) {
        self.paused_flag.store(true, Ordering::Relaxed);
    }

    /// Resume all capture threads.
    ///
    /// Clears the shared paused flag. Sets the clipboard reseed flag so
    /// the clipboard thread treats the current clipboard content as its
    /// baseline — changes made during pause (e.g. yank copying rendered
    /// narration) won't appear as events in the next recording period.
    pub fn resume(&mut self) {
        self.clipboard_reseed.store(true, Ordering::Relaxed);
        self.paused_flag.store(false, Ordering::Relaxed);
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
        self.stop_flag.store(true, Ordering::Relaxed);

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
    let stop_flag = Arc::new(AtomicBool::new(false));
    let paused_flag = Arc::new(AtomicBool::new(false));

    let editor_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let diff_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let ext_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let clipboard_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));

    // Shared file path list: written by editor_capture, read by diff_capture.
    let open_paths: Arc<Mutex<Vec<Utf8PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let editor_thread = super::editor_capture::spawn(
        config.editor_source,
        Arc::clone(&config.clock),
        Arc::clone(&stop_flag),
        Arc::clone(&paused_flag),
        cwd,
        Arc::clone(&editor_events),
        Arc::clone(&open_paths),
    );

    let diff_thread = super::diff_capture::spawn(
        Arc::clone(&config.clock),
        Arc::clone(&stop_flag),
        Arc::clone(&paused_flag),
        Arc::clone(&open_paths),
        Arc::clone(&diff_events),
    );

    let ext_thread = if let Some(ext_source) = config.ext_source {
        super::ext_capture::spawn(
            ext_source,
            Arc::clone(&config.clock),
            Arc::clone(&stop_flag),
            Arc::clone(&paused_flag),
            Arc::clone(&ext_events),
            ext_ignore_apps,
        )
    } else {
        None
    };

    let clipboard_reseed = Arc::new(AtomicBool::new(false));
    let clipboard_thread = if clipboard_capture {
        if let Some(source) = config.clipboard_source {
            super::clipboard_capture::spawn(
                source,
                Arc::clone(&config.clock),
                Arc::clone(&stop_flag),
                Arc::clone(&paused_flag),
                Arc::clone(&clipboard_reseed),
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
        stop_flag,
        paused_flag,
        clipboard_reseed,
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
