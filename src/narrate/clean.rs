//! Archive cleanup for old narration files.

use std::fs;
use std::time::{Duration, SystemTime};

use super::cache_dir;

/// Remove archived narration files older than the given duration.
pub(crate) fn clean(older_than: Duration) -> anyhow::Result<()> {
    let archive_root = cache_dir().join("archive");
    if !archive_root.exists() {
        println!("No archive directory found.");
        return Ok(());
    }

    let removed = clean_archive_dir(archive_root.as_std_path(), older_than);

    // Also clean up old clipboard staging images (referenced by path in
    // narration output, so they must outlive the narration write but not forever).
    let clip_removed = clean_flat_dir(super::clipboard_staging_dir().as_std_path(), older_than);

    let age = humantime::format_duration(older_than);
    println!(
        "Removed {removed} archived narration(s) and {clip_removed} clipboard image(s) older than {age}."
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

/// Remove files older than `older_than` from a flat (non-session-scoped) directory.
/// Returns the number of files removed.
pub(crate) fn clean_flat_dir(dir: &std::path::Path, older_than: Duration) -> u64 {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };

    let cutoff = SystemTime::now() - older_than;
    let mut removed = 0u64;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let dominated_by_cutoff = path
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .is_some_and(|mtime| mtime < cutoff);

        if dominated_by_cutoff && fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }

    removed
}
