//! Background tracking of file content changes.
//!
//! Spawns a thread that watches open files for content changes (via mtime)
//! and emits [`Event::FileDiff`] with the old and new content.
//!
//! The set of files to watch is provided by the editor capture thread via
//! a shared `Arc<Mutex<Vec<Utf8PathBuf>>>`, avoiding a redundant editor
//! query on every poll cycle.

use std::collections::HashMap;
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::Utf8PathBuf;

use super::merge::Event;
use crate::clock::Clock;

/// How often to poll for file content changes (secs).
const FILE_DIFF_POLL_SECS: u64 = 1;

/// Spawn the file diff tracking thread.
///
/// Reads the current set of open files from `open_paths` (published by
/// the editor capture thread) instead of querying the editor directly.
/// Returns the join handle. The thread pushes `FileDiff` events into
/// `events` until stopped via `control`.
pub(super) fn spawn(
    clock: Arc<dyn Clock>,
    control: Arc<super::capture::CaptureControl>,
    open_paths: Arc<Mutex<Vec<Utf8PathBuf>>>,
    events: Arc<Mutex<Vec<Event>>>,
) -> std::thread::JoinHandle<()> {
    crate::clock::spawn_clock_thread("diff", &*clock, move |clock| {
        let mut file_contents: HashMap<Utf8PathBuf, String> = HashMap::new();
        let mut file_mtimes: HashMap<Utf8PathBuf, std::time::SystemTime> = HashMap::new();

        // Snapshot initial state of whatever files are already open.
        {
            let Ok(paths) = open_paths.lock() else {
                tracing::error!("open_paths mutex poisoned: diff capture thread exiting");
                return;
            };
            for path in paths.iter() {
                if let Ok(content) = fs::read_to_string(path) {
                    if let Ok(meta) = fs::metadata(path)
                        && let Ok(mtime) = meta.modified()
                    {
                        file_mtimes.insert(path.clone(), mtime);
                    }
                    file_contents.insert(path.clone(), content);
                }
            }
        }

        'outer: loop {
            if control.wait_while_paused(&*clock) {
                break;
            }

            clock.sleep(Duration::from_secs(FILE_DIFF_POLL_SECS));

            // Read the current file list from the editor capture thread.
            let paths = {
                let Ok(guard) = open_paths.lock() else {
                    tracing::error!("open_paths mutex poisoned: diff capture thread exiting");
                    break;
                };
                guard.clone()
            };

            for path in &paths {
                let Ok(meta) = fs::metadata(path) else {
                    continue;
                };
                let Ok(mtime) = meta.modified() else {
                    continue;
                };

                let changed = file_mtimes
                    .get(path)
                    .map(|prev| *prev != mtime)
                    .unwrap_or(true);

                if !changed {
                    continue;
                }

                file_mtimes.insert(path.clone(), mtime);

                let Ok(new_content) = fs::read_to_string(path) else {
                    continue;
                };

                if let Some(old_content) = file_contents.get(path)
                    && *old_content != new_content
                {
                    let timestamp = clock.now();
                    let Ok(mut guard) = events.lock() else {
                        tracing::error!("event mutex poisoned: diff capture thread exiting");
                        break 'outer;
                    };
                    // Keep absolute path — filtering deferred to receive.
                    guard.push(Event::FileDiff {
                        timestamp,
                        path: path.clone(),
                        old: old_content.clone(),
                        new: new_content.clone(),
                    });
                }

                file_contents.insert(path.clone(), new_content);
            }
        }
    })
}
