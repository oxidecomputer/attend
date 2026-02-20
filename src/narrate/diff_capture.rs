//! Background tracking of file content changes.
//!
//! Spawns a thread that watches open files for content changes (via mtime)
//! and emits [`Event::FileDiff`] with the old and new content.

use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use camino::Utf8PathBuf;

use super::merge::Event;
use crate::state::EditorState;

/// How often to poll for file content changes (secs).
const FILE_DIFF_POLL_SECS: u64 = 1;

/// Spawn the file diff tracking thread.
///
/// Returns the join handle. The thread pushes `FileDiff` events into
/// `events` until `stop` is set.
pub(super) fn spawn(
    stop: Arc<AtomicBool>,
    cwd: Option<Utf8PathBuf>,
    events: Arc<Mutex<Vec<Event>>>,
    start: Instant,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut file_contents: HashMap<Utf8PathBuf, String> = HashMap::new();
        let mut file_mtimes: HashMap<Utf8PathBuf, std::time::SystemTime> = HashMap::new();

        // Snapshot initial state of recently active files
        if let Ok(Some(state)) = EditorState::current(cwd.as_deref(), &[]) {
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

        while !stop.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_secs(FILE_DIFF_POLL_SECS));

            // Check current editor files for changes
            let state = match EditorState::current(cwd.as_deref(), &[]) {
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
                    let display_path = file.path.as_str().to_string();
                    events.lock().unwrap().push(Event::FileDiff {
                        offset_secs,
                        path: display_path,
                        old: old_content.clone(),
                        new: new_content.clone(),
                    });
                }

                file_contents.insert(file.path.clone(), new_content);
            }
        }
    })
}
