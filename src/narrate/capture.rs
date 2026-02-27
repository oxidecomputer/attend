//! Background capture of editor state snapshots, file diffs, and external selections.
//!
//! Coordinates up to three capture threads with shared state:
//! - [`editor_capture`]: polls editor selections, emits `EditorSnapshot`,
//!   and publishes the current set of open file paths.
//! - [`diff_capture`]: reads the shared file list (instead of querying the
//!   editor independently) and watches for content changes via mtime.
//! - [`ext_capture`]: polls the platform accessibility API for selected text
//!   in external applications (e.g. iTerm2, Safari).
//!
//! Clipboard polling is managed separately: the thread is killed on pause
//! and a fresh one is spawned on resume. This avoids a race where clipboard
//! changes made while paused (e.g. yank copying rendered narration) would
//! be captured as events in the next recording period.

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

    // Clipboard thread has its own lifecycle: killed on pause, respawned on
    // resume, so that each recording period seeds from the current clipboard
    // and never observes changes made while paused.
    clipboard_stop: Arc<AtomicBool>,
    clipboard_thread: Option<thread::JoinHandle<()>>,
    clipboard_enabled: bool,
    clipboard_staging_dir: Utf8PathBuf,
}

impl CaptureHandle {
    /// Pause all capture threads.
    ///
    /// Editor, diff, and ext threads enter a sleep loop via the shared
    /// paused flag. The clipboard thread is stopped outright and its join
    /// handle released — a fresh thread is spawned on [`resume`](Self::resume).
    pub fn pause(&mut self) {
        self.paused_flag.store(true, Ordering::Relaxed);
        self.stop_clipboard();
    }

    /// Resume all capture threads.
    ///
    /// Clears the shared paused flag for editor/diff/ext and spawns a
    /// fresh clipboard polling thread that seeds from the current clipboard.
    pub fn resume(&mut self) {
        self.paused_flag.store(false, Ordering::Relaxed);
        self.spawn_clipboard();
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
        self.stop_clipboard();

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

        self.drain()
    }

    /// Stop the clipboard polling thread and join it.
    fn stop_clipboard(&mut self) {
        self.clipboard_stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.clipboard_thread.take() {
            let _ = h.join();
        }
    }

    /// Spawn a fresh clipboard polling thread with a new stop flag.
    fn spawn_clipboard(&mut self) {
        if !self.clipboard_enabled {
            return;
        }
        let Some(source) = super::clipboard_capture::ArboardClipboardSource::new() else {
            return;
        };
        let stop = Arc::new(AtomicBool::new(false));
        self.clipboard_stop = Arc::clone(&stop);
        self.clipboard_thread = super::clipboard_capture::spawn(
            Box::new(source),
            stop,
            Arc::clone(&self.clipboard_events),
            self.clipboard_staging_dir.clone(),
        );
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
        Box::new(super::editor_capture::RealEditorSource),
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

    let clipboard_stop = Arc::new(AtomicBool::new(false));
    let clipboard_thread = if clipboard_capture {
        if let Some(source) = super::clipboard_capture::ArboardClipboardSource::new() {
            super::clipboard_capture::spawn(
                Box::new(source),
                Arc::clone(&clipboard_stop),
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
        editor_events,
        diff_events,
        ext_events,
        clipboard_events,
        editor_thread: Some(editor_thread),
        diff_thread: Some(diff_thread),
        ext_thread,
        clipboard_stop,
        clipboard_thread,
        clipboard_enabled: clipboard_capture,
        clipboard_staging_dir,
    })
}
