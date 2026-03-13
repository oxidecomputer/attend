//! Archive cleanup for old narration files.

use std::fs;
use std::time::{Duration, SystemTime};

/// Remove old narration files from `archive/`, `pending/`, and clipboard staging.
///
/// Pending narrations for sessions that are no longer active will never be
/// picked up by an agent, so they get the same retention treatment as archives.
pub(crate) fn clean(older_than: Duration) -> anyhow::Result<()> {
    let narration = crate::narrate::narration_root();
    let archive_root = narration.join("archive");
    let pending_root = narration.join("pending");

    let archive_removed = clean_archive_dir(archive_root.as_std_path(), older_than);
    let pending_removed = clean_archive_dir(pending_root.as_std_path(), older_than);

    // Also clean up old clipboard staging images (referenced by path in
    // narration output, so they must outlive the narration write but not forever).
    // Now session-scoped: walk session subdirectories and prune empty ones.
    let clip_removed = clean_archive_dir(
        crate::narrate::clipboard_staging_root().as_std_path(),
        older_than,
    );

    let age = humantime::format_duration(older_than);
    println!(
        "Removed {} narration(s) ({archive_removed} archived, {pending_removed} stale pending) \
         and {clip_removed} clipboard image(s) older than {age}.",
        archive_removed + pending_removed,
    );
    Ok(())
}

/// Walk an archive root directory, removing files older than `older_than`.
/// Returns the number of files removed. Also removes empty session directories.
pub(crate) fn clean_archive_dir(archive_root: &std::path::Path, older_than: Duration) -> u64 {
    let Ok(sessions) = fs::read_dir(archive_root) else {
        return 0;
    };

    let cutoff = SystemTime::now() - older_than;
    let mut removed = 0u64;

    for entry in sessions.filter_map(|e| e.ok()) {
        let session_dir = entry.path();
        if !session_dir.is_dir() {
            continue;
        }

        let Ok(files) = fs::read_dir(&session_dir) else {
            continue;
        };

        for file in files.filter_map(|e| e.ok()) {
            let path = file.path();
            let dominated_by_cutoff = path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|mtime| mtime < cutoff);

            if dominated_by_cutoff && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }

        // Remove session directory if now empty.
        if fs::read_dir(&session_dir).is_ok_and(|mut d| d.next().is_none()) {
            let _ = fs::remove_dir(&session_dir); // Best-effort: only succeeds if empty
        }
    }

    removed
}
