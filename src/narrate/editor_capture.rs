//! Background polling of editor selections.
//!
//! Spawns a thread that polls [`EditorState`] every 100ms and emits
//! [`Event::EditorSnapshot`] whenever the file/selection set changes.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use camino::{Utf8Path, Utf8PathBuf};

use super::merge::{Event, RenderedFile};
use crate::state::{self, EditorState};
use crate::view;

/// How often to poll for editor selection changes (ms).
const EDITOR_POLL_MS: u64 = 100;

/// Spawn the editor polling thread.
///
/// Returns the join handle. The thread pushes `EditorSnapshot` events into
/// `events` until `stop` is set.
pub(super) fn spawn(
    stop: Arc<AtomicBool>,
    cwd: Option<Utf8PathBuf>,
    events: Arc<Mutex<Vec<Event>>>,
    start: Instant,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut prev_files: Option<Vec<state::FileEntry>> = None;

        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(EDITOR_POLL_MS));

            let state = match EditorState::current(cwd.as_deref(), &[]) {
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

            events.lock().unwrap().push(Event::EditorSnapshot {
                offset_secs,
                files,
                rendered,
            });
        }
    })
}

/// Render file entries into `RenderedFile` entries for an editor snapshot.
fn render_snapshot_files(files: &[state::FileEntry], cwd: Option<&Utf8Path>) -> Vec<RenderedFile> {
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
