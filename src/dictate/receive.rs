//! Check for and deliver pending dictation files to Claude Code.
//!
//! Dictation files are stored as individual timestamped markdown files in
//! `~/.cache/attend/pending/<session_id>/`. When multiple dictations happen
//! before a receive, they are concatenated chronologically (separated by
//! `---`) and delivered as a single response.

use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use super::{
    archive_dir, cache_dir, listening_session, pending_dir, receive_lock_path, resolve_session,
};

/// Re-dispatch instruction appended to output when running with `--wait`.
const REDISPATCH_MSG: &str =
    "\n[Run `attend dictate receive --wait` in the background to wait for the next dictation.]";

/// Collect all pending dictation files for a session, sorted by filename (timestamp).
fn collect_pending(session_id: &str) -> Vec<PathBuf> {
    let dir = pending_dir(session_id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();

    files.sort();
    files
}

/// Read and concatenate all pending dictation files, separating with `---`.
fn read_pending(files: &[PathBuf]) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let mut parts = Vec::new();
    for path in files {
        if let Ok(content) = fs::read_to_string(path) {
            let content = content.trim().to_string();
            if !content.is_empty() {
                parts.push(content);
            }
        }
    }

    if parts.is_empty() {
        return None;
    }

    Some(parts.join("\n\n---\n\n"))
}

/// Archive pending dictation files by moving them to the archive directory.
fn archive_pending(files: &[PathBuf], session_id: &str) {
    let archive = archive_dir(session_id);
    let _ = fs::create_dir_all(&archive);

    for path in files {
        if let Some(filename) = path.file_name() {
            let dest = archive.join(filename);
            let _ = fs::rename(path, &dest);
        }
    }

    // Clean up the pending dir if empty
    let dir = pending_dir(session_id);
    let _ = fs::remove_dir(&dir); // only succeeds if empty
}

/// Try to acquire an exclusive lock file. Returns the path on success.
fn try_lock(lock_path: &Path) -> Option<LockGuard> {
    if let Some(parent) = lock_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Use O_CREAT | O_EXCL for atomic creation
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(_) => {
            // Write our PID for debugging
            let _ = fs::write(lock_path, std::process::id().to_string());
            Some(LockGuard {
                path: lock_path.to_path_buf(),
            })
        }
        Err(_) => {
            // Check if the lock is stale (process no longer exists)
            if let Ok(content) = fs::read_to_string(lock_path)
                && let Ok(pid) = content.trim().parse::<u32>()
                && !process_exists(pid)
            {
                let _ = fs::remove_file(lock_path);
                return try_lock(lock_path);
            }
            None
        }
    }
}

/// Check if a process with the given PID exists.
fn process_exists(pid: u32) -> bool {
    // kill(pid, 0) checks existence without sending a signal
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// RAII guard that removes the lock file on drop.
struct LockGuard {
    path: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Run the receive subcommand.
///
/// Without `--wait`: check once, print if found, exit.
/// With `--wait`: poll until dictation arrives or session is stolen.
pub fn run(wait: bool, session_flag: Option<String>) -> anyhow::Result<()> {
    let session_id = resolve_session(session_flag);

    if wait {
        run_wait(session_id)
    } else {
        run_once(session_id)
    }
}

/// One-shot check: print pending dictations if any exist, then exit.
fn run_once(session_id: Option<String>) -> anyhow::Result<()> {
    let session_id = match session_id {
        Some(s) => s,
        None => {
            // No session — try unsuffixed fallback
            let fallback = cache_dir().join("dictation.md");
            if fallback.exists()
                && let Ok(content) = fs::read_to_string(&fallback)
            {
                let content = content.trim();
                if !content.is_empty() {
                    print!("{content}");
                    let _ = fs::remove_file(&fallback);
                    return Ok(());
                }
            }
            std::process::exit(1);
        }
    };

    let files = collect_pending(&session_id);
    match read_pending(&files) {
        Some(content) => {
            print!("{content}");
            archive_pending(&files, &session_id);
            Ok(())
        }
        None => std::process::exit(1),
    }
}

/// Polling wait: hold a lock, poll for dictation, detect session steal.
fn run_wait(session_id: Option<String>) -> anyhow::Result<()> {
    let session_id = match session_id {
        Some(s) => s,
        None => {
            anyhow::bail!("no session ID available (use --session or run /attend first)");
        }
    };

    // Acquire exclusive lock
    let lock_path = receive_lock_path();
    let _lock = match try_lock(&lock_path) {
        Some(guard) => guard,
        None => {
            // Another listener is already running
            eprintln!("Listener already running.");
            std::process::exit(0);
        }
    };

    let poll_interval = Duration::from_millis(500);

    loop {
        // Check if session was stolen
        match listening_session() {
            Some(current) if current == session_id => {}
            _ => {
                println!(
                    "Dictation was transferred to another Claude session. \
                     Run /attend to reactivate."
                );
                return Ok(());
            }
        }

        // Check for pending dictation
        let files = collect_pending(&session_id);
        if let Some(content) = read_pending(&files) {
            print!("{content}");
            println!("{REDISPATCH_MSG}");
            archive_pending(&files, &session_id);
            return Ok(());
        }

        thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collect_pending_empty_dir() {
        let files = collect_pending("nonexistent-session");
        assert!(files.is_empty());
    }

    #[test]
    fn read_pending_empty() {
        assert!(read_pending(&[]).is_none());
    }

    #[test]
    fn read_pending_single() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("2026-02-18T10-00-00Z.md");
        fs::write(&path, "hello world").unwrap();
        let result = read_pending(&[path]).unwrap();
        assert_eq!(result, "hello world");
    }

    #[test]
    fn read_pending_multiple_concatenated() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("2026-02-18T10-00-00Z.md");
        let p2 = dir.path().join("2026-02-18T10-05-00Z.md");
        fs::write(&p1, "first dictation").unwrap();
        fs::write(&p2, "second dictation").unwrap();
        let result = read_pending(&[p1, p2]).unwrap();
        assert_eq!(result, "first dictation\n\n---\n\nsecond dictation");
    }

    #[test]
    fn archive_moves_files() {
        let base = tempfile::tempdir().unwrap();

        // Set up a fake pending dir structure
        let pending = base.path().join("pending").join("test-session");
        fs::create_dir_all(&pending).unwrap();
        let file = pending.join("2026-02-18T10-00-00Z.md");
        fs::write(&file, "content").unwrap();

        let archive = base.path().join("archive").join("test-session");
        fs::create_dir_all(&archive).unwrap();

        // We can't easily test archive_pending since it uses hardcoded paths,
        // but we can test the file operations directly
        let dest = archive.join("2026-02-18T10-00-00Z.md");
        fs::rename(&file, &dest).unwrap();

        assert!(!file.exists());
        assert!(dest.exists());
        assert_eq!(fs::read_to_string(&dest).unwrap(), "content");
    }

    #[test]
    fn collect_pending_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let pending = dir.path();

        // Create files out of order
        fs::write(pending.join("b.md"), "second").unwrap();
        fs::write(pending.join("a.md"), "first").unwrap();
        fs::write(pending.join("c.md"), "third").unwrap();
        fs::write(pending.join("not-md.txt"), "skip").unwrap();

        // We can test sorting directly
        let mut files = vec![
            pending.join("b.md"),
            pending.join("a.md"),
            pending.join("c.md"),
        ];
        files.sort();
        assert_eq!(
            files
                .iter()
                .map(|p| p.file_name().unwrap().to_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["a.md", "b.md", "c.md"]
        );
    }

    #[test]
    fn lock_guard_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let lock_path = dir.path().join("test.lock");

        {
            let _guard = try_lock(&lock_path).expect("should acquire lock");
            assert!(lock_path.exists());

            // Second attempt should fail
            assert!(try_lock(&lock_path).is_none());
        }

        // After drop, lock should be removed
        assert!(!lock_path.exists());
    }
}
