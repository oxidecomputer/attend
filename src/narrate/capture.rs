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
use std::time::Instant;

use camino::Utf8PathBuf;

use super::merge::Event;

/// Handle for the background editor/diff/ext polling threads.
pub(crate) struct CaptureHandle {
    stop_flag: Arc<AtomicBool>,
    editor_events: Arc<Mutex<Vec<Event>>>,
    diff_events: Arc<Mutex<Vec<Event>>>,
    ext_events: Arc<Mutex<Vec<Event>>>,
    editor_thread: Option<thread::JoinHandle<()>>,
    diff_thread: Option<thread::JoinHandle<()>>,
    ext_thread: Option<thread::JoinHandle<()>>,
}

impl CaptureHandle {
    /// Drain accumulated events without stopping threads.
    pub fn drain(&self) -> (Vec<Event>, Vec<Event>, Vec<Event>) {
        let editor = std::mem::take(&mut *self.editor_events.lock().unwrap());
        let diff = std::mem::take(&mut *self.diff_events.lock().unwrap());
        let ext = std::mem::take(&mut *self.ext_events.lock().unwrap());
        (editor, diff, ext)
    }

    /// Signal stop and collect remaining results.
    pub fn collect(mut self) -> (Vec<Event>, Vec<Event>, Vec<Event>) {
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

        self.drain()
    }
}

/// Start background threads for editor polling, file diff tracking, and
/// external selection capture.
///
/// The editor capture thread publishes open file paths into `open_paths`,
/// which the diff capture thread reads instead of querying the editor
/// independently. This eliminates a redundant database query + offset
/// resolution per diff poll cycle.
///
/// The external capture thread polls the platform accessibility API for
/// selected text in the focused application. It is not spawned if the
/// platform has no backend or accessibility permission is not granted.
///
/// Pass `None` for `cwd` to keep paths absolute (filtering deferred to receive).
pub(crate) fn start(
    cwd: Option<Utf8PathBuf>,
    ext_ignore_apps: Vec<String>,
) -> anyhow::Result<CaptureHandle> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let editor_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let diff_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let ext_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));

    // Shared file path list: written by editor_capture, read by diff_capture.
    let open_paths: Arc<Mutex<Vec<Utf8PathBuf>>> = Arc::new(Mutex::new(Vec::new()));

    let editor_thread = super::editor_capture::spawn(
        Arc::clone(&stop_flag),
        cwd,
        Arc::clone(&editor_events),
        Arc::clone(&open_paths),
        start,
    );

    let diff_thread = super::diff_capture::spawn(
        Arc::clone(&stop_flag),
        Arc::clone(&open_paths),
        Arc::clone(&diff_events),
        start,
    );

    let ext_thread = super::ext_capture::spawn(
        Arc::clone(&stop_flag),
        Arc::clone(&ext_events),
        start,
        ext_ignore_apps,
    );

    Ok(CaptureHandle {
        stop_flag,
        editor_events,
        diff_events,
        ext_events,
        editor_thread: Some(editor_thread),
        diff_thread: Some(diff_thread),
        ext_thread,
    })
}
