//! Background capture of editor state snapshots and file diffs.
//!
//! Spawns two threads: one polls editor selections every 100ms and emits
//! [`Event::EditorSnapshot`] on change, the other watches open files for
//! content changes and emits [`Event::FileDiff`].

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use super::merge::{Event, RenderedFile};
use crate::state::{self, EditorState};
use crate::view;

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
pub(crate) fn start(cwd: Option<PathBuf>) -> anyhow::Result<CaptureHandle> {
    let stop_flag = Arc::new(AtomicBool::new(false));
    let start = Instant::now();

    let editor_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));
    let diff_events: Arc<Mutex<Vec<Event>>> = Arc::new(Mutex::new(Vec::new()));

    // Editor state polling thread
    let stop_ed = Arc::clone(&stop_flag);
    let ed_cwd = cwd.clone();
    let ed_events = Arc::clone(&editor_events);
    let editor_thread = thread::spawn(move || {
        let mut prev_files: Option<Vec<state::FileEntry>> = None;

        while !stop_ed.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));

            let state = match EditorState::current(ed_cwd.as_deref(), &[]) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            // Check if file entries changed
            if prev_files.as_ref() == Some(&state.files) {
                continue;
            }

            let files = state.files;
            prev_files = Some(files.clone());

            let offset_secs = start.elapsed().as_secs_f64();
            // Pass None for cwd so paths stay absolute — filtering deferred to receive.
            let rendered = render_snapshot_files(&files, None);

            ed_events.lock().unwrap().push(Event::EditorSnapshot {
                offset_secs,
                files,
                rendered,
            });
        }
    });

    // File diff tracking thread
    let stop_diff = Arc::clone(&stop_flag);
    let diff_cwd = cwd;
    let df_events = Arc::clone(&diff_events);
    let diff_thread = thread::spawn(move || {
        let mut file_contents: HashMap<PathBuf, String> = HashMap::new();
        let mut file_mtimes: HashMap<PathBuf, std::time::SystemTime> = HashMap::new();

        // Snapshot initial state of recently active files
        if let Ok(Some(state)) = EditorState::current(diff_cwd.as_deref(), &[]) {
            for file in &state.files {
                if let Ok(content) = fs::read_to_string(&file.path) {
                    if let Ok(meta) = fs::metadata(&file.path)
                        && let Ok(mtime) = meta.modified()
                    {
                        file_mtimes.insert(file.path.clone(), mtime);
                    }
                    file_contents.insert(file.path.clone(), content);
                }
            }
        }

        while !stop_diff.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(1));

            // Check current editor files for changes
            let state = match EditorState::current(diff_cwd.as_deref(), &[]) {
                Ok(Some(s)) => s,
                _ => continue,
            };

            for file in &state.files {
                let Ok(meta) = fs::metadata(&file.path) else {
                    continue;
                };
                let Ok(mtime) = meta.modified() else {
                    continue;
                };

                let changed = file_mtimes
                    .get(&file.path)
                    .map(|prev| *prev != mtime)
                    .unwrap_or(true);

                if !changed {
                    continue;
                }

                file_mtimes.insert(file.path.clone(), mtime);

                let Ok(new_content) = fs::read_to_string(&file.path) else {
                    continue;
                };

                if let Some(old_content) = file_contents.get(&file.path)
                    && *old_content != new_content
                {
                    let offset_secs = start.elapsed().as_secs_f64();
                    // Keep absolute path — filtering deferred to receive.
                    let display_path = file.path.to_string_lossy().to_string();
                    df_events.lock().unwrap().push(Event::FileDiff {
                        offset_secs,
                        path: display_path,
                        old: old_content.clone(),
                        new: new_content.clone(),
                    });
                }

                file_contents.insert(file.path.clone(), new_content);
            }
        }
    });

    Ok(CaptureHandle {
        stop_flag,
        editor_events,
        diff_events,
        editor_thread: Some(editor_thread),
        diff_thread: Some(diff_thread),
    })
}

/// Render file entries into `RenderedFile` entries with relative paths.
fn render_snapshot_files(files: &[state::FileEntry], cwd: Option<&Path>) -> Vec<RenderedFile> {
    let mut rendered = Vec::new();

    for file in files {
        if file.selections.is_empty() {
            continue;
        }

        let Ok(payload) = view::render_json(std::slice::from_ref(file), cwd, view::Extent::Exact)
        else {
            continue;
        };

        for vf in payload.files {
            for group in &vf.groups {
                rendered.push(RenderedFile {
                    path: vf.path.clone(),
                    content: group.content.clone(),
                    first_line: group.first_line.get() as u32,
                });
            }
        }
    }

    rendered
}
