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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use camino::Utf8PathBuf;

use super::merge::Event;
use crate::clock::Clock;

/// How often to poll for file content changes (secs).
const FILE_DIFF_POLL_SECS: u64 = 1;

/// Sleep interval when paused (ms).
const PAUSED_POLL_MS: u64 = 500;

/// Spawn the file diff tracking thread.
///
/// Reads the current set of open files from `open_paths` (published by
/// the editor capture thread) instead of querying the editor directly.
/// Returns the join handle. The thread pushes `FileDiff` events into
/// `events` until `stop` is set.
pub(super) fn spawn(
    clock: Arc<dyn Clock>,
    stop: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    open_paths: Arc<Mutex<Vec<Utf8PathBuf>>>,
    events: Arc<Mutex<Vec<Event>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let mut file_contents: HashMap<Utf8PathBuf, String> = HashMap::new();
        let mut file_mtimes: HashMap<Utf8PathBuf, std::time::SystemTime> = HashMap::new();

        // Snapshot initial state of whatever files are already open.
        {
            let paths = open_paths.lock().unwrap();
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

        while !stop.load(Ordering::Relaxed) {
            if paused.load(Ordering::Relaxed) {
                clock.sleep(Duration::from_millis(PAUSED_POLL_MS));
                continue;
            }

            clock.sleep(Duration::from_secs(FILE_DIFF_POLL_SECS));

            // Read the current file list from the editor capture thread.
            let paths = open_paths.lock().unwrap().clone();

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
                    // Keep absolute path — filtering deferred to receive.
                    let display_path = path.as_str().to_string();
                    events.lock().unwrap().push(Event::FileDiff {
                        timestamp,
                        path: display_path,
                        old: old_content.clone(),
                        new: new_content.clone(),
                    });
                }

                file_contents.insert(path.clone(), new_content);
            }
        }
    })
}
