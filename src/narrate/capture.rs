//! Background capture of editor state snapshots, file diffs, and external selections.
//!
//! Coordinates up to three capture threads with shared state:
//! - [`editor_capture`]: polls editor selections, emits `EditorSnapshot`,
//!   and publishes the current set of open file paths.
//! - [`diff_capture`]: reads the shared file list (instead of querying the
//!   editor independently) and watches for content changes via mtime.
//! - [`ext_capture`]: polls the platform accessibility API for selected text
//!   in external applications (e.g. iTerm2, Safari).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;

use camino::Utf8PathBuf;

use super::merge::Event;

/// Handle for the background editor/diff/ext/clipboard polling threads.
pub(crate) struct CaptureHandle {
    stop_flag: Arc<AtomicBool>,
    paused_flag: Arc<AtomicBool>,
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
    /// Pause all capture threads (skip polling, sleep at longer intervals).
    pub fn pause(&self) {
        self.paused_flag.store(true, Ordering::Relaxed);
    }

    /// Resume all capture threads.
    pub fn resume(&self) {
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
    cwd: Option<Utf8PathBuf>,
    ext_ignore_apps: Vec<String>,
    clipboard_capture: bool,
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
        Arc::clone(&stop_flag),
        Arc::clone(&paused_flag),
        cwd,
        Arc::clone(&editor_events),
        Arc::clone(&open_paths),
    );

    let diff_thread = super::diff_capture::spawn(
        Arc::clone(&stop_flag),
        Arc::clone(&paused_flag),
        Arc::clone(&open_paths),
        Arc::clone(&diff_events),
    );

    let ext_thread = super::ext_capture::spawn(
        Arc::clone(&stop_flag),
        Arc::clone(&paused_flag),
        Arc::clone(&ext_events),
        ext_ignore_apps,
    );

    let clipboard_thread = if clipboard_capture {
        super::clipboard_capture::spawn(
            Arc::clone(&stop_flag),
            Arc::clone(&paused_flag),
            Arc::clone(&clipboard_events),
        )
    } else {
        None
    };

    Ok(CaptureHandle {
        stop_flag,
        paused_flag,
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
