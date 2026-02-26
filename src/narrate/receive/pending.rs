//! Pending narration file lifecycle: collection, archival, and pruning.

use std::fs;
use std::path::PathBuf;

use camino::Utf8Path;

use crate::config::Config;
use crate::narrate::{archive_dir, pending_dir};
use crate::state::SessionId;

/// Collect all pending narration files for a session, sorted by filename (timestamp).
///
/// Also collects files from the `_local` directory (narrations captured when
/// no agent session was active), so they are delivered when a session starts.
pub(crate) fn collect_pending(session_id: &SessionId) -> Vec<PathBuf> {
    let mut files = collect_pending_dir(&pending_dir(Some(session_id)));
    // Also collect from _local (no-session narrations).
    files.extend(collect_pending_dir(&pending_dir(None)));
    files.sort();
    files
}

/// Collect `.json` files from a single pending directory.
pub(super) fn collect_pending_dir(dir: &Utf8Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

/// Archive pending narration files by moving them to the archive directory.
///
/// Files from both the session directory and `_local` are archived under the
/// session's archive directory. Empty source directories are cleaned up.
pub(crate) fn archive_pending(files: &[PathBuf], session_id: &SessionId) {
    let archive = archive_dir(Some(session_id));
    // Best-effort archival: non-critical for narration delivery.
    let _ = fs::create_dir_all(&archive);

    for path in files {
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let dest = archive.join(filename);
            let _ = fs::rename(path, dest.as_std_path());
        }
    }

    // Best-effort: only succeeds if empty.
    let dir = pending_dir(Some(session_id));
    let _ = fs::remove_dir(&dir);
    // Also clean _local if empty (files may have come from there).
    let local_dir = pending_dir(None);
    let _ = fs::remove_dir(&local_dir);
}

/// Prune old narrations from both `archive/` and `pending/`.
///
/// Pending narrations for sessions that are no longer active will never be
/// picked up by an agent. Applying the same retention policy prevents them
/// from accumulating indefinitely.
///
/// No-op if retention is `"forever"`.
pub(crate) fn auto_prune(config: &Config) {
    if let Some(retention) = config.retention_duration() {
        let narration = crate::narrate::narration_root();
        for dir in ["archive", "pending"] {
            let root = narration.join(dir);
            if root.exists() {
                crate::narrate::clean::clean_archive_dir(root.as_std_path(), retention);
            }
        }
    }
}
