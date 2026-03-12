use std::fs;

use camino::Utf8PathBuf;

use crate::state::{self, SessionId};

/// Root of the per-session state tree: `<cache>/sessions/`.
fn sessions_dir() -> Option<Utf8PathBuf> {
    Some(state::cache_dir()?.join("sessions"))
}

/// Per-session cache: tracks what was last emitted to a given session for deduplication.
pub(super) fn session_cache_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(
        sessions_dir()?
            .join("cache")
            .join(format!("{session_id}.json")),
    )
}

/// Path to the "displaced" marker for a given session.
///
/// A displaced session is one that is no longer the active listener — either
/// because another session took over or because narration was explicitly
/// deactivated. Displaced sessions should not auto-reclaim narration.
fn displaced_marker_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(sessions_dir()?.join("displaced").join(session_id.as_str()))
}

/// Check whether this session has been displaced (stolen or deactivated).
pub(super) fn session_displaced(session_id: &SessionId) -> bool {
    displaced_marker_path(session_id).is_some_and(|p| p.exists())
}

/// Record that this session has been displaced.
pub(crate) fn mark_session_displaced(session_id: &SessionId) {
    if let Some(path) = displaced_marker_path(session_id) {
        let _ = fs::create_dir_all(path.parent().unwrap());
        let _ = fs::write(&path, "");
    }
}

/// Remove all marker files from the `sessions/displaced/` and `sessions/activated/` directories,
/// and prune stale dedup caches from `sessions/cache/`.
///
/// Called on session start to prevent unbounded accumulation of
/// files from old sessions. Marker files are unconditionally removed
/// (they are transient). Cache files are pruned by mtime: living
/// sessions update their cache on every hook call, so only dead
/// sessions go stale.
pub(super) fn clean_session_markers() {
    let Some(sessions) = sessions_dir() else {
        return;
    };
    for subdir in ["displaced", "activated"] {
        let dir = sessions.join(subdir);
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let _ = fs::remove_file(entry.path());
        }
    }

    // Prune stale dedup caches (older than 24h). Active sessions touch
    // their cache on every prompt, so they always have a fresh mtime.
    let cache_dir = sessions.join("cache");
    if let Ok(entries) = fs::read_dir(&cache_dir) {
        let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(24 * 60 * 60);
        for entry in entries.flatten() {
            let stale = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|mtime| mtime < cutoff);
            if stale {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

/// Clear the "displaced" marker, e.g. when `/attend` re-activates.
pub(crate) fn clear_session_displaced(session_id: &SessionId) {
    if let Some(path) = displaced_marker_path(session_id) {
        let _ = fs::remove_file(&path); // Best-effort
    }
}

/// Path to the "activated" marker for a given session.
fn activated_marker_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(sessions_dir()?.join("activated").join(session_id.as_str()))
}

/// Check whether this session has ever activated `/attend`.
pub(super) fn session_was_activated(session_id: &SessionId) -> bool {
    activated_marker_path(session_id).is_some_and(|p| p.exists())
}

/// Record that this session has activated `/attend`.
pub(super) fn mark_session_activated(session_id: &SessionId) {
    if let Some(path) = activated_marker_path(session_id) {
        let _ = fs::create_dir_all(path.parent().unwrap());
        let _ = fs::write(&path, "");
    }
}
