use std::fs;

use camino::Utf8PathBuf;

use crate::state::{self, SessionId};

/// Per-session cache: tracks what was last emitted to a given session for deduplication.
pub(super) fn session_cache_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(state::cache_dir()?.join(format!("cache-{session_id}.json")))
}

/// Path to the "session moved" notification marker for a given session.
fn moved_marker_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(state::cache_dir()?.join(format!("moved-{session_id}")))
}

/// Check whether this session has already been notified of a session move.
pub(super) fn session_moved_already_notified(session_id: &SessionId) -> bool {
    moved_marker_path(session_id).is_some_and(|p| p.exists())
}

/// Record that this session has been notified of a session move.
pub(super) fn mark_session_moved_notified(session_id: &SessionId) {
    if let Some(path) = moved_marker_path(session_id) {
        // Intentionally ignored: marker is advisory, not critical.
        let _ = fs::write(&path, "");
    }
}

/// Remove all `moved-*` and `activated-*` marker files from the cache directory.
///
/// Called on session start to prevent unbounded accumulation of
/// marker files from old sessions.
pub(super) fn clean_session_markers() {
    let Some(cache) = state::cache_dir() else {
        return;
    };
    let Ok(entries) = fs::read_dir(&cache) else {
        return;
    };
    for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str()
            && (name.starts_with("moved-") || name.starts_with("activated-"))
        {
            // Intentionally ignored: stale markers are harmless.
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Clear the "session moved" marker, e.g. when `/attend` re-activates.
pub(crate) fn clear_session_moved_marker(session_id: &SessionId) {
    if let Some(path) = moved_marker_path(session_id) {
        let _ = fs::remove_file(&path); // Best-effort
    }
}

/// Path to the "activated" marker for a given session.
fn activated_marker_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(state::cache_dir()?.join(format!("activated-{session_id}")))
}

/// Check whether this session has ever activated `/attend`.
pub(super) fn session_was_activated(session_id: &SessionId) -> bool {
    activated_marker_path(session_id).is_some_and(|p| p.exists())
}

/// Record that this session has activated `/attend`.
pub(super) fn mark_session_activated(session_id: &SessionId) {
    if let Some(path) = activated_marker_path(session_id) {
        // Intentionally ignored: marker is advisory, not critical.
        let _ = fs::write(&path, "");
    }
}
