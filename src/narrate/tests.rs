use std::io::Write;

use camino::Utf8PathBuf;

use crate::state::SessionId;

use super::*;

// -- process_alive tests --

/// The current process is reported as alive.
#[test]
fn process_alive_current_pid() {
    let pid = std::process::id() as i32;
    assert!(process_alive(pid));
}

/// A very high PID (i32::MAX) is reported as not alive.
#[test]
fn process_alive_dead_pid() {
    // PID 0 is the kernel's swapper; sending signal to it from unprivileged
    // code should fail with EPERM, but process_alive uses kill(pid,0)==0,
    // so EPERM means "exists but no permission" — which returns false on
    // the raw check.  Use a very high PID that almost certainly doesn't exist.
    assert!(!process_alive(i32::MAX));
}

// -- is_lock_stale tests (via record module) --

/// A lock file containing the current process PID is not stale.
#[test]
fn is_lock_stale_with_live_pid() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    let pid = std::process::id();
    std::fs::write(&lock, pid.to_string()).unwrap();
    assert!(!record::is_lock_stale(&lock));
}

/// A lock file containing a dead PID (i32::MAX) is stale.
#[test]
fn is_lock_stale_with_dead_pid() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    std::fs::write(&lock, i32::MAX.to_string()).unwrap();
    assert!(record::is_lock_stale(&lock));
}

/// A nonexistent lock file is not reported as stale.
#[test]
fn is_lock_stale_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("nonexistent.lock")).unwrap();
    assert!(!record::is_lock_stale(&lock));
}

/// A lock file with non-numeric content is not reported as stale.
#[test]
fn is_lock_stale_invalid_content() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    std::fs::write(&lock, "not-a-number").unwrap();
    assert!(!record::is_lock_stale(&lock));
}

// -- clean tests --

/// Archive cleanup removes files older than the threshold and empty session dirs.
#[test]
fn clean_removes_old_files() {
    let dir = tempfile::tempdir().unwrap();
    let archive_root = dir.path().join("archive");
    let session_dir = archive_root.join("test-session");
    std::fs::create_dir_all(&session_dir).unwrap();

    // Create an old file (set mtime to 10 days ago).
    let old_file = session_dir.join("old.json");
    let mut f = std::fs::File::create(&old_file).unwrap();
    f.write_all(b"old").unwrap();
    drop(f);
    // We can't easily set mtime in pure Rust, so instead test with a very
    // long threshold that keeps the file and a zero threshold that removes it.

    // With a huge threshold, nothing is removed.
    let count = clean_archive_dir(&archive_root, std::time::Duration::from_secs(365 * 86400));
    assert_eq!(count, 0);
    assert!(old_file.exists());

    // With zero threshold, everything is removed (all files are older than "now").
    let count = clean_archive_dir(&archive_root, std::time::Duration::ZERO);
    assert_eq!(count, 1);
    assert!(!old_file.exists());
    // Session directory should be removed since it's empty.
    assert!(!session_dir.exists());
}

/// When no files expire (huge threshold), directories and files are untouched.
#[test]
fn clean_preserves_nonempty_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let archive_root = dir.path().join("archive");
    let session_dir = archive_root.join("test-session");
    std::fs::create_dir_all(&session_dir).unwrap();

    std::fs::write(session_dir.join("a.json"), "data-a").unwrap();
    std::fs::write(session_dir.join("b.json"), "data-b").unwrap();

    // Huge threshold: nothing is old enough to remove.
    let count = clean_archive_dir(&archive_root, std::time::Duration::from_secs(365 * 86400));
    assert_eq!(count, 0, "no files should be removed");
    assert!(session_dir.exists(), "session dir should be preserved");
    assert!(session_dir.join("a.json").exists(), "a.json should survive");
    assert!(session_dir.join("b.json").exists(), "b.json should survive");
}

/// All expired files are removed, and the now-empty session directory is cleaned up.
#[test]
fn clean_removes_all_and_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let archive_root = dir.path().join("archive");
    let session_dir = archive_root.join("test-session");
    std::fs::create_dir_all(&session_dir).unwrap();

    std::fs::write(session_dir.join("a.json"), "old").unwrap();
    std::fs::write(session_dir.join("b.json"), "old").unwrap();

    // Zero threshold: everything is "expired."
    let count = clean_archive_dir(&archive_root, std::time::Duration::ZERO);
    assert_eq!(count, 2);
    assert!(!session_dir.exists(), "empty dir should be removed");
}

/// An explicit --session flag overrides any listening session.
#[test]
fn resolve_session_flag_takes_precedence() {
    let result = resolve_session(Some("my-session".to_string()));
    assert_eq!(result, Some(SessionId::from("my-session")));
}

/// resolve_session(None) reads the listening file. Writing a known session ID
/// to the file, then calling resolve_session(None), returns that session ID.
/// Whitespace is trimmed.
#[test]
fn resolve_session_reads_listening_file() {
    use crate::state::listening_path;

    let path = listening_path().expect("cache dir should be available in test");

    // Save any existing content so we can restore it.
    let original = std::fs::read_to_string(&path).ok();

    // Write a known session ID (with whitespace to verify trimming).
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "  test-session-xyz\n").unwrap();
    let result = resolve_session(None);

    // Restore original state.
    match original {
        Some(content) => std::fs::write(&path, content).unwrap(),
        None => {
            let _ = std::fs::remove_file(&path);
        }
    }

    assert_eq!(result, Some(SessionId::from("test-session-xyz")));
}

/// The cache directory path ends with "attend".
#[test]
fn cache_dir_is_under_attend() {
    let dir = cache_dir();
    assert!(dir.ends_with("attend"));
}

/// The pending directory path includes the session ID.
#[test]
fn pending_dir_includes_session() {
    let sid = SessionId::from("abc-123");
    let dir = pending_dir(Some(&sid));
    assert!(dir.ends_with("pending/abc-123") || dir.ends_with("pending\\abc-123"));
}

/// The archive directory path includes the session ID.
#[test]
fn archive_dir_includes_session() {
    let sid = SessionId::from("abc-123");
    let dir = archive_dir(Some(&sid));
    assert!(dir.ends_with("archive/abc-123") || dir.ends_with("archive\\abc-123"));
}

// -- No-session (_local) fallback tests --

/// When no session ID is provided, pending_dir uses the `_local` fallback.
#[test]
fn pending_dir_falls_back_to_local() {
    let dir = pending_dir(None);
    assert!(
        dir.ends_with("pending/_local") || dir.ends_with("pending\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

/// When no session ID is provided, archive_dir uses the `_local` fallback.
#[test]
fn archive_dir_falls_back_to_local() {
    let dir = archive_dir(None);
    assert!(
        dir.ends_with("archive/_local") || dir.ends_with("archive\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

/// When no session ID is provided, browser_staging_dir uses the `_local` fallback.
#[test]
fn browser_staging_dir_falls_back_to_local() {
    let dir = browser_staging_dir(None);
    assert!(
        dir.ends_with("browser-staging/_local") || dir.ends_with("browser-staging\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

/// `browser_staging_dir(Some(sid))` still includes the session ID.
#[test]
fn browser_staging_dir_includes_session() {
    let sid = SessionId::from("sess-99");
    let dir = browser_staging_dir(Some(&sid));
    assert!(
        dir.ends_with("browser-staging/sess-99") || dir.ends_with("browser-staging\\sess-99"),
        "expected session ID, got: {dir}"
    );
}
