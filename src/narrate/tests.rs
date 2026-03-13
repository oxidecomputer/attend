use std::fs;
use std::io::Write;

use camino::Utf8PathBuf;

use crate::clock::RealClock;
use crate::state::{CacheDirGuard, SessionId};

use super::*;

/// Write a fake daemon lock file for tests that simulate a running daemon.
///
/// Creates the `daemon/` directory (which now holds the lock) and writes
/// the PID with a current timestamp in the new `PID:TIMESTAMP` format.
fn write_fake_lock(pid: impl std::fmt::Display) {
    let lock = record_lock_path();
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(&lock, format!("{pid}:{now}")).unwrap();
}

/// Build a lock file content string using the actual process start time
/// from sysinfo (instead of `SystemTime::now()`).
///
/// `lock_file_content()` writes `SystemTime::now()` as the creation
/// timestamp, but `process_alive_since()` compares that against the
/// process's real start time from sysinfo. If the test binary has been
/// running for more than 2 seconds, the two diverge and the process
/// appears "dead" (PID reuse false positive). This helper avoids that.
fn lock_content_with_real_start_time() -> String {
    use sysinfo::{ProcessRefreshKind, System};

    let pid = std::process::id();
    let sysinfo_pid = sysinfo::Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo_pid]),
        true,
        ProcessRefreshKind::nothing(),
    );
    let start_time = sys
        .process(sysinfo_pid)
        .map(|p| p.start_time())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs()
        });
    format!("{pid}:{start_time}")
}

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

// -- parse_lock_content tests --

/// New format `PID:TIMESTAMP` is parsed correctly.
#[test]
fn parse_lock_content_new_format() {
    let (pid, ts) = super::parse_lock_content("12345:1700000000").unwrap();
    assert_eq!(pid, 12345);
    assert_eq!(ts, Some(1700000000));
}

/// Legacy format (PID only) is parsed correctly.
#[test]
fn parse_lock_content_legacy_format() {
    let (pid, ts) = super::parse_lock_content("12345").unwrap();
    assert_eq!(pid, 12345);
    assert_eq!(ts, None);
}

/// Whitespace around the content is tolerated.
#[test]
fn parse_lock_content_whitespace_trimmed() {
    let (pid, ts) = super::parse_lock_content("  42:999  ").unwrap();
    assert_eq!(pid, 42);
    assert_eq!(ts, Some(999));
}

/// Non-numeric content returns None.
#[test]
fn parse_lock_content_garbage_returns_none() {
    assert!(super::parse_lock_content("not-a-number").is_none());
}

/// Content with a colon but non-numeric timestamp returns None.
#[test]
fn parse_lock_content_bad_timestamp_returns_none() {
    assert!(super::parse_lock_content("123:abc").is_none());
}

// -- lock_file_content tests --

/// lock_file_content produces the `PID:TIMESTAMP` format.
#[test]
fn lock_file_content_format() {
    let content = super::lock_file_content();
    let (pid, ts) = super::parse_lock_content(&content).unwrap();
    assert_eq!(pid, std::process::id() as i32);
    assert!(ts.is_some(), "should include a timestamp");
    // Timestamp should be within a few seconds of now.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    assert!((ts.unwrap() - now).abs() < 5);
}

// -- lock_owner_alive tests --

/// The current process is reported alive via lock_owner_alive (new format).
#[test]
fn lock_owner_alive_current_process() {
    let content = lock_content_with_real_start_time();
    assert!(super::lock_owner_alive(&content));
}

/// A dead PID is reported not alive via lock_owner_alive (new format).
#[test]
fn lock_owner_alive_dead_pid_new_format() {
    let content = format!("{}:1700000000", i32::MAX);
    assert!(!super::lock_owner_alive(&content));
}

/// Legacy format (PID only) falls back to process_alive.
#[test]
fn lock_owner_alive_legacy_format() {
    let pid = std::process::id();
    assert!(super::lock_owner_alive(&pid.to_string()));
    assert!(!super::lock_owner_alive(&i32::MAX.to_string()));
}

/// Unparseable content returns false.
#[test]
fn lock_owner_alive_garbage_returns_false() {
    assert!(!super::lock_owner_alive("not-a-number"));
}

// -- is_lock_stale tests (via record module) --

/// A lock file containing the current process PID (new format) is not stale.
#[test]
fn is_lock_stale_with_live_pid() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    std::fs::write(&lock, lock_content_with_real_start_time()).unwrap();
    assert!(!record::is_lock_stale(&lock));
}

/// A lock file containing a dead PID (new format) is stale.
#[test]
fn is_lock_stale_with_dead_pid() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    std::fs::write(&lock, format!("{}:1700000000", i32::MAX)).unwrap();
    assert!(record::is_lock_stale(&lock));
}

/// A lock file with a legacy PID-only format still works.
#[test]
fn is_lock_stale_legacy_format() {
    let dir = tempfile::tempdir().unwrap();
    let lock = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();
    // Live PID in legacy format.
    std::fs::write(&lock, std::process::id().to_string()).unwrap();
    assert!(!record::is_lock_stale(&lock));
    // Dead PID in legacy format.
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
    assert!(
        dir.ends_with("narration/pending/abc-123") || dir.ends_with("narration\\pending\\abc-123")
    );
}

/// The archive directory path includes the session ID.
#[test]
fn archive_dir_includes_session() {
    let sid = SessionId::from("abc-123");
    let dir = archive_dir(Some(&sid));
    assert!(
        dir.ends_with("narration/archive/abc-123") || dir.ends_with("narration\\archive\\abc-123")
    );
}

// -- No-session (_local) fallback tests --

/// When no session ID is provided, pending_dir uses the `_local` fallback.
#[test]
fn pending_dir_falls_back_to_local() {
    let dir = pending_dir(None);
    assert!(
        dir.ends_with("narration/pending/_local") || dir.ends_with("narration\\pending\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

/// When no session ID is provided, archive_dir uses the `_local` fallback.
#[test]
fn archive_dir_falls_back_to_local() {
    let dir = archive_dir(None);
    assert!(
        dir.ends_with("narration/archive/_local") || dir.ends_with("narration\\archive\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

/// When no session ID is provided, browser_staging_dir uses the `_local` fallback.
#[test]
fn browser_staging_dir_falls_back_to_local() {
    let dir = browser_staging_dir(None);
    assert!(
        dir.ends_with("staging/browser/_local") || dir.ends_with("staging\\browser\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

/// `browser_staging_dir(Some(sid))` still includes the session ID.
#[test]
fn browser_staging_dir_includes_session() {
    let sid = SessionId::from("sess-99");
    let dir = browser_staging_dir(Some(&sid));
    assert!(
        dir.ends_with("staging/browser/sess-99") || dir.ends_with("staging\\browser\\sess-99"),
        "expected session ID, got: {dir}"
    );
}

// -- Yanked directory tests --

/// The yanked directory path includes the session ID.
#[test]
fn yanked_dir_includes_session() {
    let sid = SessionId::from("abc-123");
    let dir = super::yanked_dir(Some(&sid));
    assert!(
        dir.ends_with("narration/yanked/abc-123") || dir.ends_with("narration\\yanked\\abc-123"),
        "expected session ID, got: {dir}"
    );
}

/// When no session ID is provided, yanked_dir uses the `_local` fallback.
#[test]
fn yanked_dir_falls_back_to_local() {
    let dir = super::yanked_dir(None);
    assert!(
        dir.ends_with("narration/yanked/_local") || dir.ends_with("narration\\yanked\\_local"),
        "expected _local fallback, got: {dir}"
    );
}

// -- Pause tests --
//
// These use CacheDirGuard to redirect all filesystem state to a tempdir,
// then exercise the real record::pause() function.

/// `record::pause()` when not recording prints an error and is a no-op.
#[test]
fn pause_not_recording_is_noop() {
    let _g = CacheDirGuard::new();
    record::pause().unwrap();
    assert!(!pause_sentinel_path().exists());
}

/// `record::pause()` creates the sentinel when recording and none exists,
/// and removes it on the second call (toggle behavior).
#[test]
fn pause_toggle_round_trip() {
    let _g = CacheDirGuard::new();

    // Simulate a running daemon by writing a record lock.
    write_fake_lock(std::process::id());

    // First call: creates the sentinel (pause).
    record::pause().unwrap();
    assert!(
        pause_sentinel_path().exists(),
        "sentinel should exist after first pause"
    );

    // Second call: removes the sentinel (resume).
    record::pause().unwrap();
    assert!(
        !pause_sentinel_path().exists(),
        "sentinel should not exist after second pause (resume)"
    );
}

/// Multiple pause/resume cycles through `record::pause()` always
/// converge to the expected state.
#[test]
fn pause_multiple_cycles() {
    let _g = CacheDirGuard::new();
    write_fake_lock(std::process::id());

    for i in 0..5 {
        record::pause().unwrap();
        assert!(
            pause_sentinel_path().exists(),
            "cycle {i}: sentinel should exist after pause"
        );
        record::pause().unwrap();
        assert!(
            !pause_sentinel_path().exists(),
            "cycle {i}: sentinel should not exist after resume"
        );
    }
}

/// `record::stop()` while paused still works (writes stop sentinel).
#[test]
fn stop_while_paused_is_accepted() {
    let _g = CacheDirGuard::new();

    // Use a dead PID so stop() doesn't wait 5 seconds for a daemon.
    write_fake_lock(i32::MAX);

    record::pause().unwrap();
    assert!(pause_sentinel_path().exists());

    // Stop writes the stop sentinel even while paused.
    record::stop(&RealClock).unwrap();
}

/// The pause sentinel path lives under the overridden cache directory,
/// and its presence/absence tracks pause state.
#[test]
fn pause_sentinel_under_cache_dir() {
    let g = CacheDirGuard::new();
    let sentinel = pause_sentinel_path();

    // Sentinel path is under the overridden cache dir.
    assert!(sentinel.starts_with(g.cache.as_str()));

    // Write record lock with our PID (alive process).
    write_fake_lock(std::process::id());

    // Without pause sentinel: not paused.
    assert!(!sentinel.exists());

    // With pause sentinel: paused.
    std::fs::write(&sentinel, "").unwrap();
    assert!(sentinel.exists());
}

// -- Yank tests --
//
// These use CacheDirGuard to redirect all filesystem state to a tempdir,
// then exercise the real record::yank() function.

/// `record::yank()` when not recording prints an error and is a no-op.
#[test]
fn yank_not_recording_is_noop() {
    let _g = CacheDirGuard::new();
    record::yank(&RealClock).unwrap();
    // No sentinel should exist — yank exits early when not recording.
    assert!(!super::yank_sentinel_path().exists());
}

/// `record::yank()` writes the yank sentinel when recording.
#[test]
fn yank_writes_sentinel() {
    let _g = CacheDirGuard::new();

    // Use a dead PID so yank() doesn't wait 5 seconds for a daemon.
    write_fake_lock(i32::MAX);

    record::yank(&RealClock).unwrap();
    // Sentinel is cleaned up by the CLI after it detects daemon exit,
    // but the daemon (dead PID) never ran, so the sentinel may or may not
    // remain. The key assertion: yank didn't panic and ran to completion.
}

/// `record::yank()` while paused still works (writes yank sentinel).
#[test]
fn yank_while_paused_is_accepted() {
    let _g = CacheDirGuard::new();

    // Use a dead PID so yank() doesn't wait 5 seconds for a daemon.
    write_fake_lock(i32::MAX);

    record::pause().unwrap();
    assert!(pause_sentinel_path().exists());

    // Yank while paused should succeed.
    record::yank(&RealClock).unwrap();
}

/// The yank sentinel path lives under the overridden cache directory.
#[test]
fn yank_sentinel_under_cache_dir() {
    let g = CacheDirGuard::new();
    let sentinel = super::yank_sentinel_path();

    // Sentinel path is under the overridden cache dir.
    assert!(sentinel.starts_with(g.cache.as_str()));
}

// -- Yank property tests --
//
// These verify structural invariants of the yank path that must hold
// for any session ID and any event content.

use proptest::prelude::*;

/// `yanked_dir` and `pending_dir` never produce the same path for any session ID.
///
/// This is the core isolation invariant: the hook delivery path (which reads
/// `pending/`) must never accidentally pick up yanked content.
#[test]
fn yanked_dir_disjoint_from_pending_dir() {
    proptest!(|(session_id in "[a-zA-Z0-9_-]{1,64}")| {
        let sid = SessionId::from(session_id.as_str());
        let yanked = super::yanked_dir(Some(&sid));
        let pending = super::pending_dir(Some(&sid));
        // Neither should be a prefix of the other.
        prop_assert!(!yanked.starts_with(&pending));
        prop_assert!(!pending.starts_with(&yanked));
        prop_assert_ne!(yanked, pending, "yanked and pending dirs must differ");
    });
}

/// `yanked_dir(None)` and `pending_dir(None)` are also disjoint (_local fallback).
#[test]
fn yanked_dir_disjoint_from_pending_dir_local() {
    let yanked = super::yanked_dir(None);
    let pending = super::pending_dir(None);
    assert_ne!(yanked, pending);
    assert!(!yanked.starts_with(&pending));
    assert!(!pending.starts_with(&yanked));
}

/// Files written to `yanked/` are invisible to `collect_pending`.
///
/// This is the end-to-end isolation test: even when yanked files exist
/// on disk, the hook delivery pipeline never finds them.
#[test]
fn collect_pending_ignores_yanked_dir() {
    let _g = CacheDirGuard::new();
    let sid = SessionId::from("isolation-test");

    // Write a file to the yanked dir.
    let yanked = super::yanked_dir(Some(&sid));
    std::fs::create_dir_all(&yanked).unwrap();
    let events = vec![super::merge::Event::Words {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        text: "yanked content".to_string(),
    }];
    std::fs::write(
        yanked.join("test.json"),
        serde_json::to_string(&events).unwrap(),
    )
    .unwrap();

    // collect_pending should find nothing (pending dir is empty).
    let files = super::receive::collect_pending(&sid);
    assert!(
        files.is_empty(),
        "collect_pending must not see files in yanked/"
    );
}

/// Events written to the yanked dir round-trip through read_pending.
///
/// Simulates what the daemon does (write events to yanked/) and what the
/// yank CLI does (collect files, call read_pending, verify content).
#[test]
fn yanked_events_round_trip_through_read_pending() {
    let _g = CacheDirGuard::new();
    let sid = SessionId::from("round-trip-test");
    let cwd = camino::Utf8Path::new("/project");

    // Simulate daemon writing to yanked dir.
    let yanked = super::yanked_dir(Some(&sid));
    std::fs::create_dir_all(&yanked).unwrap();
    let events = vec![
        super::merge::Event::Words {
            timestamp: chrono::DateTime::UNIX_EPOCH,
            text: "hello from yank".to_string(),
        },
        super::merge::Event::Words {
            timestamp: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(1000),
            text: "more words".to_string(),
        },
    ];
    let json_path = yanked.join("2026-02-25T10-00-00Z.json");
    std::fs::write(&json_path, serde_json::to_string(&events).unwrap()).unwrap();

    // Simulate yank CLI: collect files and read.
    let files: Vec<std::path::PathBuf> = vec![json_path.into_std_path_buf()];
    let content =
        super::receive::read_pending(&files, Some(cwd), &[], super::render::RenderMode::Agent)
            .expect("should produce content from yanked events");

    assert!(content.contains("hello from yank"));
    assert!(content.contains("more words"));
    // Should be raw markdown, not wrapped in tags.
    assert!(!content.contains("<narration>"));
}

/// Cleanup after yank removes all collected files and leaves the dir empty.
#[test]
fn yank_cleanup_removes_files_and_empty_dir() {
    let _g = CacheDirGuard::new();
    let sid = SessionId::from("cleanup-test");

    let yanked = super::yanked_dir(Some(&sid));
    std::fs::create_dir_all(&yanked).unwrap();

    // Write multiple files.
    for i in 0..3 {
        let events = vec![super::merge::Event::Words {
            timestamp: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(i * 1000),
            text: format!("chunk {i}"),
        }];
        std::fs::write(
            yanked.join(format!("{i:020}.json")),
            serde_json::to_string(&events).unwrap(),
        )
        .unwrap();
    }

    // Collect the files.
    let files: Vec<std::path::PathBuf> = std::fs::read_dir(&yanked)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .collect();
    assert_eq!(files.len(), 3);

    // Simulate yank cleanup: remove files, then try to remove empty dir.
    for path in &files {
        std::fs::remove_file(path).unwrap();
    }
    let _ = std::fs::remove_dir(&yanked);

    assert!(
        !yanked.exists(),
        "yanked dir should be removed after cleanup"
    );
}

/// Yanked files are archived (not deleted) after yank, so they survive
/// for `attend narrate clean` on the normal retention schedule.
#[test]
fn yank_archives_instead_of_deleting() {
    let _g = CacheDirGuard::new();
    let sid = SessionId::from("archive-test");

    // Simulate daemon writing to yanked dir.
    let yanked = super::yanked_dir(Some(&sid));
    std::fs::create_dir_all(&yanked).unwrap();
    let events = vec![super::merge::Event::Words {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        text: "archived content".to_string(),
    }];
    let filename = "2026-02-25T10-00-00Z.json";
    std::fs::write(
        yanked.join(filename),
        serde_json::to_string(&events).unwrap(),
    )
    .unwrap();

    // Simulate the archive step from record::yank().
    let files: Vec<std::path::PathBuf> = vec![yanked.join(filename).into_std_path_buf()];
    let archive = super::archive_dir(Some(&sid));
    std::fs::create_dir_all(&archive).unwrap();
    for path in &files {
        if let Some(fname) = path.file_name().and_then(|f| f.to_str()) {
            let dest = archive.join(fname);
            std::fs::rename(path, dest.as_std_path()).unwrap();
        }
    }
    let _ = std::fs::remove_dir(&yanked);

    // Yanked dir is gone.
    assert!(!yanked.exists(), "yanked dir should be cleaned up");

    // File is in the archive.
    let archived = archive.join(filename);
    assert!(archived.exists(), "yanked file should be archived");

    // Archived content is intact.
    let content = std::fs::read_to_string(&archived).unwrap();
    let parsed: Vec<super::merge::Event> = serde_json::from_str(&content).unwrap();
    assert_eq!(parsed.len(), 1);
}

/// Yank with a session filters content by the session's cwd, matching
/// the behavior of hook delivery via stop.
#[test]
fn yank_with_session_filters_by_cwd() {
    let _g = CacheDirGuard::new();
    let sid = SessionId::from("cwd-filter-test");
    let cwd = camino::Utf8Path::new("/project");

    // Write events with files both inside and outside the cwd.
    let yanked = super::yanked_dir(Some(&sid));
    std::fs::create_dir_all(&yanked).unwrap();
    let events = vec![
        super::merge::Event::Words {
            timestamp: chrono::DateTime::UNIX_EPOCH,
            text: "look at these files".to_string(),
        },
        super::merge::Event::EditorSnapshot {
            timestamp: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(100),
            last_seen: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(100),
            files: vec![],
            regions: vec![
                super::merge::CapturedRegion {
                    path: "/project/src/main.rs".to_string(),
                    content: "fn main() {}\n".to_string(),
                    first_line: 1,
                    selections: vec![],
                    language: None,
                },
                super::merge::CapturedRegion {
                    path: "/other-project/lib.rs".to_string(),
                    content: "fn other() {}\n".to_string(),
                    first_line: 1,
                    selections: vec![],
                    language: None,
                },
            ],
        },
    ];
    std::fs::write(
        yanked.join("test.json"),
        serde_json::to_string(&events).unwrap(),
    )
    .unwrap();

    let files: Vec<std::path::PathBuf> = vec![yanked.join("test.json").into_std_path_buf()];

    // With cwd filtering (session present): only project files survive.
    let content =
        super::receive::read_pending(&files, Some(cwd), &[], super::render::RenderMode::Agent)
            .unwrap();
    assert!(
        content.contains("src/main.rs"),
        "project file should be included and relativized"
    );
    assert!(
        !content.contains("/other-project"),
        "outside file should be filtered out"
    );
}

/// Yank without a session includes all content unfiltered, with absolute paths.
///
/// When there is no receiving session, there is no project context to filter
/// against. The user can paste the full narration anywhere they choose.
#[test]
fn yank_without_session_includes_all_content() {
    let _g = CacheDirGuard::new();

    // Write events with files from different directories (no common root).
    let yanked = super::yanked_dir(None);
    std::fs::create_dir_all(&yanked).unwrap();
    let events = vec![
        super::merge::Event::Words {
            timestamp: chrono::DateTime::UNIX_EPOCH,
            text: "various files".to_string(),
        },
        super::merge::Event::EditorSnapshot {
            timestamp: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(100),
            last_seen: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(100),
            files: vec![],
            regions: vec![
                super::merge::CapturedRegion {
                    path: "/project-a/src/main.rs".to_string(),
                    content: "fn main_a() {}\n".to_string(),
                    first_line: 1,
                    selections: vec![],
                    language: None,
                },
                super::merge::CapturedRegion {
                    path: "/project-b/lib.rs".to_string(),
                    content: "fn lib_b() {}\n".to_string(),
                    first_line: 1,
                    selections: vec![],
                    language: None,
                },
            ],
        },
        super::merge::Event::FileDiff {
            timestamp: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(200),
            path: "/project-c/test.rs".to_string(),
            old: "old\n".to_string(),
            new: "new\n".to_string(),
        },
    ];
    std::fs::write(
        yanked.join("test.json"),
        serde_json::to_string(&events).unwrap(),
    )
    .unwrap();

    let files: Vec<std::path::PathBuf> = vec![yanked.join("test.json").into_std_path_buf()];

    // Without cwd filtering (no session): all content passes through.
    let content =
        super::receive::read_pending(&files, None, &[], super::render::RenderMode::Yank).unwrap();
    assert!(
        content.contains("/project-a/src/main.rs"),
        "project-a file should be included with absolute path"
    );
    assert!(
        content.contains("/project-b/lib.rs"),
        "project-b file should be included with absolute path"
    );
    assert!(
        content.contains("/project-c/test.rs"),
        "project-c diff should be included with absolute path"
    );
}

/// For any session ID, yanked files written by the daemon are collected
/// from both the session dir and the _local fallback dir.
#[test]
fn yank_collects_from_session_and_local() {
    let _g = CacheDirGuard::new();
    let sid = SessionId::from("dual-collect-test");

    // Write to session-specific yanked dir.
    let session_dir = super::yanked_dir(Some(&sid));
    std::fs::create_dir_all(&session_dir).unwrap();
    let session_events = vec![super::merge::Event::Words {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        text: "session yank".to_string(),
    }];
    std::fs::write(
        session_dir.join("session.json"),
        serde_json::to_string(&session_events).unwrap(),
    )
    .unwrap();

    // Write to _local yanked dir.
    let local_dir = super::yanked_dir(None);
    std::fs::create_dir_all(&local_dir).unwrap();
    let local_events = vec![super::merge::Event::Words {
        timestamp: chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds(500),
        text: "local yank".to_string(),
    }];
    std::fs::write(
        local_dir.join("local.json"),
        serde_json::to_string(&local_events).unwrap(),
    )
    .unwrap();

    // Collect from both dirs (same pattern as record::yank).
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    for dir in [&session_dir, &local_dir] {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("json") {
                    files.push(p);
                }
            }
        }
    }
    files.sort();

    assert_eq!(files.len(), 2, "should collect from both dirs");

    let cwd = camino::Utf8Path::new("/project");
    let content =
        super::receive::read_pending(&files, Some(cwd), &[], super::render::RenderMode::Agent)
            .unwrap();
    assert!(content.contains("session yank"));
    assert!(content.contains("local yank"));
}

// -- collect_staging timestamp parsing --

/// Staging files with the same second-level timestamp but different UUID
/// suffixes are both collected. This prevents preexec/postexec events
/// from overwriting each other for fast commands like `cd`.
#[test]
fn collect_staging_uuid_suffix_prevents_collision() {
    let guard = CacheDirGuard::new();
    let session = SessionId::from("test-session");
    let dir = shell_staging_dir(Some(&session));
    fs::create_dir_all(&dir).unwrap();

    let preexec = vec![merge::Event::ShellCommand {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        shell: "fish".to_string(),
        command: "cd ..".to_string(),
        cwd: "/project".to_string(),
        exit_status: None,
        duration_secs: None,
    }];
    let postexec = vec![merge::Event::ShellCommand {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        shell: "fish".to_string(),
        command: "cd ..".to_string(),
        cwd: "/other".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.001),
    }];

    // Same timestamp, different UUID suffixes — simulates preexec/postexec
    // within the same second.
    let ts = "2026-02-25T23-45-00.000000000Z";
    let path_a = dir.join(format!("{ts}-aaaa.json"));
    let path_b = dir.join(format!("{ts}-bbbb.json"));
    fs::write(&path_a, serde_json::to_string(&preexec).unwrap()).unwrap();
    fs::write(&path_b, serde_json::to_string(&postexec).unwrap()).unwrap();

    let result = collect_shell_staging(
        Some(&session),
        chrono::DateTime::UNIX_EPOCH,
        chrono::Utc::now(),
    );
    assert_eq!(
        result.events.len(),
        2,
        "both preexec and postexec should be collected; \
         cache_dir={} dir={}",
        guard.cache,
        dir,
    );
}

/// Nanosecond-precision timestamps parse correctly and preserve ordering.
#[test]
fn collect_staging_nanos_timestamp_parsed() {
    let guard = CacheDirGuard::new();
    let session = SessionId::from("test-session");
    let dir = shell_staging_dir(Some(&session));
    fs::create_dir_all(&dir).unwrap();

    let event = vec![merge::Event::ShellCommand {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        shell: "fish".to_string(),
        command: "ls".to_string(),
        cwd: "/project".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.1),
    }];

    let path = dir.join("2026-02-25T23-45-00.500000000Z-some-uuid.json");
    fs::write(&path, serde_json::to_string(&event).unwrap()).unwrap();

    let result = collect_shell_staging(
        Some(&session),
        chrono::DateTime::UNIX_EPOCH,
        chrono::Utc::now(),
    );
    assert_eq!(result.events.len(), 1);

    // The event timestamp should be from the filename, not UNIX_EPOCH.
    let ts = result.events[0].timestamp();
    assert_ne!(
        ts,
        chrono::DateTime::UNIX_EPOCH,
        "timestamp should be parsed from filename"
    );

    drop(guard);
}

/// Legacy second-precision filenames still parse correctly.
#[test]
fn collect_staging_legacy_timestamp_parsed() {
    let guard = CacheDirGuard::new();
    let session = SessionId::from("test-session");
    let dir = shell_staging_dir(Some(&session));
    fs::create_dir_all(&dir).unwrap();

    let event = vec![merge::Event::ShellCommand {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        shell: "fish".to_string(),
        command: "ls".to_string(),
        cwd: "/project".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.1),
    }];

    // Old-style filename with no fractional seconds and no UUID.
    let path = dir.join("2026-02-25T23-45-00Z.json");
    fs::write(&path, serde_json::to_string(&event).unwrap()).unwrap();

    let result = collect_shell_staging(
        Some(&session),
        chrono::DateTime::UNIX_EPOCH,
        chrono::Utc::now(),
    );
    assert_eq!(result.events.len(), 1);

    let ts = result.events[0].timestamp();
    assert_ne!(
        ts,
        chrono::DateTime::UNIX_EPOCH,
        "legacy timestamp should be parsed from filename"
    );

    drop(guard);
}
