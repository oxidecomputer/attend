//! Event filtering and path-scoping for project-directory boundaries.
//!
//! Events captured with absolute paths are filtered to the current project
//! directory (and any configured `include_dirs`). Events outside scope are
//! replaced with [`Event::Redacted`] markers, and surviving paths are
//! relativized.

use camino::{Utf8Path, Utf8PathBuf};

use crate::narrate::merge::{Event, RedactedKind};
use crate::util::path_included;

/// Filter events to only include files under `cwd` or any `include_dirs`.
///
/// Events that fall outside scope are replaced with [`Event::Redacted`]
/// markers. Adjacent markers of the same kind are then collapsed, with
/// file-based kinds (EditorSnapshot, FileDiff) deduplicated by path.
pub(super) fn filter_events(events: &mut Vec<Event>, cwd: &Utf8Path, include_dirs: &[Utf8PathBuf]) {
    let mut filtered = Vec::with_capacity(events.len());
    for event in events.drain(..) {
        match event {
            Event::Words { .. }
            | Event::ExternalSelection { .. }
            | Event::BrowserSelection { .. }
            | Event::ClipboardSelection { .. }
            | Event::Redacted { .. } => filtered.push(event),
            Event::EditorSnapshot {
                timestamp,
                last_seen,
                mut files,
                mut regions,
            } => {
                let dropped: Vec<String> = regions
                    .iter()
                    .filter(|r| !path_included(&r.path, cwd, include_dirs))
                    .map(|r| r.path.clone())
                    .collect();
                regions.retain(|r| path_included(&r.path, cwd, include_dirs));
                files.retain(|f| path_included(f.path.as_str(), cwd, include_dirs));
                if !regions.is_empty() {
                    filtered.push(Event::EditorSnapshot {
                        timestamp,
                        last_seen,
                        files,
                        regions,
                    });
                }
                if !dropped.is_empty() {
                    filtered.push(Event::Redacted {
                        timestamp,
                        kind: RedactedKind::EditorSnapshot,
                        keys: dropped,
                    });
                }
            }
            Event::FileDiff {
                timestamp,
                path,
                old,
                new,
            } => {
                if path_included(&path, cwd, include_dirs) {
                    filtered.push(Event::FileDiff {
                        timestamp,
                        path,
                        old,
                        new,
                    });
                } else {
                    filtered.push(Event::Redacted {
                        timestamp,
                        kind: RedactedKind::FileDiff,
                        keys: vec![path],
                    });
                }
            }
            Event::ShellCommand {
                timestamp,
                shell,
                command,
                cwd: cmd_cwd,
                exit_status,
                duration_secs,
            } => {
                if path_included(&cmd_cwd, cwd, include_dirs) {
                    filtered.push(Event::ShellCommand {
                        timestamp,
                        shell,
                        command,
                        cwd: cmd_cwd,
                        exit_status,
                        duration_secs,
                    });
                } else {
                    filtered.push(Event::Redacted {
                        timestamp,
                        kind: RedactedKind::ShellCommand,
                        keys: vec![command],
                    });
                }
            }
        }
    }

    collapse_redacted(&mut filtered);
    *events = filtered;
}

/// Collapse runs of adjacent `Redacted` events into the minimal summary.
///
/// Within a contiguous run of `Redacted` events, events are grouped by kind
/// (freely reordered) with keys deduplicated. A run like `✂ file, ✂ edit,
/// ✂ file` becomes `✂ 2 files, ✂ edit`.
fn collapse_redacted(events: &mut Vec<Event>) {
    use std::collections::BTreeMap;

    let mut result = Vec::with_capacity(events.len());
    let mut i = 0;

    while i < events.len() {
        if !matches!(&events[i], Event::Redacted { .. }) {
            result.push(std::mem::replace(
                &mut events[i],
                // Placeholder; will be discarded when we replace `*events`.
                Event::Words {
                    timestamp: chrono::DateTime::UNIX_EPOCH,
                    text: String::new(),
                },
            ));
            i += 1;
            continue;
        }

        // Collect the entire run of consecutive Redacted events.
        let mut by_kind: BTreeMap<RedactedKind, (chrono::DateTime<chrono::Utc>, Vec<String>)> =
            BTreeMap::new();
        while i < events.len() {
            if let Event::Redacted {
                timestamp,
                kind,
                keys,
            } = std::mem::replace(
                &mut events[i],
                Event::Words {
                    timestamp: chrono::DateTime::UNIX_EPOCH,
                    text: String::new(),
                },
            ) {
                let entry = by_kind
                    .entry(kind)
                    .or_insert_with(|| (timestamp, Vec::new()));
                entry.1.extend(keys);
                i += 1;
            } else {
                // Not Redacted — end of run.
                break;
            }
        }

        // Emit one Redacted per kind, with deduplicated keys.
        for (kind, (timestamp, mut keys)) in by_kind {
            keys.sort();
            keys.dedup();
            result.push(Event::Redacted {
                timestamp,
                kind,
                keys,
            });
        }
    }

    *events = result;
}

/// Rewrite absolute paths to be relative to `cwd`.
pub(super) fn relativize_events(events: &mut [Event], cwd: &Utf8Path) {
    for event in events.iter_mut() {
        match event {
            Event::EditorSnapshot { regions, .. } => {
                for region in regions.iter_mut() {
                    region.path = relativize_str(&region.path, cwd);
                }
            }
            Event::FileDiff { path, .. } => {
                *path = relativize_str(path, cwd);
            }
            Event::ShellCommand { cwd: cmd_cwd, .. } => {
                *cmd_cwd = relativize_str(cmd_cwd, cwd);
            }
            // External/browser/clipboard selections and redacted markers have no paths to relativize.
            Event::Words { .. }
            | Event::ExternalSelection { .. }
            | Event::BrowserSelection { .. }
            | Event::ClipboardSelection { .. }
            | Event::Redacted { .. } => {}
        }
    }
}

/// Strip a cwd prefix from a path string, returning the relative form.
fn relativize_str(path: &str, cwd: &Utf8Path) -> String {
    let p = Utf8Path::new(path);
    match p.strip_prefix(cwd) {
        Ok(rel) => rel.as_str().to_string(),
        Err(_) => path.to_string(),
    }
}
