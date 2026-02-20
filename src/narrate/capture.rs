//! Background capture of editor state snapshots and file diffs.
//!
//! Coordinates two independent capture threads:
//! - [`editor_capture`]: polls editor selections, emits `EditorSnapshot`
//! - [`diff_capture`]: watches file content changes, emits `FileDiff`

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use camino::Utf8PathBuf;

use super::merge::Event;

/// Handle for the background editor/diff polling threads.
pub(crate) struct CaptureHandle {
    stop_flag: Arc<AtomicBool>,
    editor_events: Arc<Mutex<Vec<Event>>>,
    diff_events: Arc<Mutex<Vec<Event>>>,
    editor_thread: Option<thread::JoinHandle<()>>,
    diff_thread: Option<thread::JoinHandle<()>>,
}

impl CaptureHandle {
    /// Drain accumulated events without stopping threads.
    pub fn drain(&self) -> (Vec<Event>, Vec<Event>) {
        let editor = std::mem::take(&mut *self.editor_events.lock().unwrap());
        let diff = std::mem::take(&mut *self.diff_events.lock().unwrap());
        (editor, diff)
    }

    /// Signal stop and collect remaining results.
    pub fn collect(mut self) -> (Vec<Event>, Vec<Event>) {
        self.stop_flag.store(true, Ordering::Relaxed);

        if let Some(h) = self.editor_thread.take() {
            let _ = h.join();
        }
        if let Some(h) = self.diff_thread.take() {
            let _ = h.join();
        }

        self.drain()
    }
}

/// Start background threads for editor polling and file diff tracking.
///
/// Pass `None` for `cwd` to keep paths absolute (filtering deferred to receive).
pub(crate) fn start(cwd: Option<Utf8PathBuf>) -> anyhow::Result<CaptureHandle> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let editor_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let diff_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));

    let editor_thread = super::editor_capture::spawn(
        Arc::clone(&stop_flag),
        cwd.clone(),
        Arc::clone(&editor_events),
        start,
    );

    let diff_thread =
        super::diff_capture::spawn(Arc::clone(&stop_flag), cwd, Arc::clone(&diff_events), start);

    Ok(CaptureHandle {
        stop_flag,
        editor_events,
        diff_events,
        editor_thread: Some(editor_thread),
        diff_thread: Some(diff_thread),
    })
}
