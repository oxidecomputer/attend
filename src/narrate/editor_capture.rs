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

/// Minimum dwell time before emitting a cursor-only snapshot (ms).
///
/// Rapid file scanning generates many low-value cursor positions. We only
/// emit a cursor-only snapshot once the cursor has rested at a position for
/// at least this long. Snapshots with real selections (highlights) are
/// emitted immediately since they indicate intentional pointing.
const CURSOR_DWELL_MS: u64 = 500;

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
        // Pending cursor-only snapshot awaiting dwell timeout.
        let mut pending_cursor: Option<(Instant, Vec<state::FileEntry>)> = None;

        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(EDITOR_POLL_MS));

            // Flush a dwelled cursor-only snapshot if enough time has passed.
            if let Some((changed_at, ref pending_files)) = pending_cursor
                && changed_at.elapsed() >= Duration::from_millis(CURSOR_DWELL_MS)
            {
                let offset_secs = start.elapsed().as_secs_f64();
                let rendered = render_snapshot_files(pending_files, None);
                events.lock().unwrap().push(Event::EditorSnapshot {
                    offset_secs,
                    files: pending_files.clone(),
                    rendered,
                });
                pending_cursor = None;
            }

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

            let cursor_only = files
                .iter()
                .all(|f| f.selections.iter().all(|s| s.is_cursor_like()));

            if cursor_only {
                // Defer emission until the cursor dwells at this position.
                pending_cursor = Some((Instant::now(), files));
            } else {
                // Real selections are always intentional — emit immediately.
                pending_cursor = None;
                let offset_secs = start.elapsed().as_secs_f64();
                let rendered = render_snapshot_files(&files, None);
                events.lock().unwrap().push(Event::EditorSnapshot {
                    offset_secs,
                    files,
                    rendered,
                });
            }
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

        let extent = view::Extent::Lines {
            before: 1,
            after: 1,
        };
        let Ok(payload) = view::render_json(std::slice::from_ref(file), cwd, extent) else {
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
