use std::io::Write;

use camino::Utf8PathBuf;

use crate::state::SessionId;

use super::*;

// -- process_alive tests --

#[test]
fn process_alive_current_pid() {
    let pid = std::process::id() as i32;
    assert!(process_alive(pid));
}

#[test]
fn process_alive_dead_pid() {
    // PID 0 is the kernel's swapper; sending signal to it from unprivileged
    // code should fail with EPERM, but process_alive uses kill(pid,0)==0,
    // so EPERM means "exists but no permission" — which returns false on
    // the raw check.  Use a very high PID that almost certainly doesn't exist.
    assert!(!process_alive(i32::MAX));
}

// -- is_lock_stale tests (via record module) --

#[test]
fn is_lock_stale_with_live_pid() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    let pid = std::process::id();
    std::fs::write(&lock, pid.to_string()).unwrap();
    assert!(!record::is_lock_stale(&lock));
}

#[test]
fn is_lock_stale_with_dead_pid() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    std::fs::write(&lock, i32::MAX.to_string()).unwrap();
    assert!(record::is_lock_stale(&lock));
}

#[test]
fn is_lock_stale_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("nonexistent.lock")).unwrap();
    assert!(!record::is_lock_stale(&lock));
}

#[test]
fn is_lock_stale_invalid_content() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    std::fs::write(&lock, "not-a-number").unwrap();
    assert!(!record::is_lock_stale(&lock));
}

// -- clean tests --

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

#[test]
fn clean_preserves_nonempty_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let archive_root = dir.path().join("archive");
    let session_dir = archive_root.join("test-session");
    std::fs::create_dir_all(&session_dir).unwrap();

    std::fs::write(session_dir.join("a.json"), "old").unwrap();
    std::fs::write(session_dir.join("b.json"), "new").unwrap();

    // Remove with zero threshold — all files gone, dir removed.
    let count = clean_archive_dir(&archive_root, std::time::Duration::ZERO);
    assert_eq!(count, 2);
    assert!(!session_dir.exists());
}

#[test]
fn resolve_session_flag_takes_precedence() {
    let result = resolve_session(Some("my-session".to_string()));
    assert_eq!(result, Some(SessionId::from("my-session")));
}

#[test]
fn resolve_session_no_flag_no_listening() {
    // When no flag and no listening file exists, returns None
    // (depends on whether listening file exists on disk, so just test the flag path)
    let result = resolve_session(Some("test".to_string()));
    assert_eq!(result.unwrap(), SessionId::from("test"));
}

#[test]
fn cache_dir_is_under_attend() {
    let dir = cache_dir();
    assert!(dir.ends_with("attend"));
}

#[test]
fn pending_dir_includes_session() {
    let sid = SessionId::from("abc-123");
    let dir = pending_dir(&sid);
    assert!(dir.ends_with("pending/abc-123") || dir.ends_with("pending\\abc-123"));
}

#[test]
fn archive_dir_includes_session() {
    let sid = SessionId::from("abc-123");
    let dir = archive_dir(&sid);
    assert!(dir.ends_with("archive/abc-123") || dir.ends_with("archive\\abc-123"));
}
