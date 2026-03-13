use std::fs;

use crate::state::CacheDirGuard;

use super::*;

// -- DaemonCommand round-trip tests --

/// Every DaemonCommand variant round-trips through Display and FromStr.
#[test]
fn daemon_command_round_trip_all_variants() {
    let variants = [
        DaemonCommand::Stop,
        DaemonCommand::Flush,
        DaemonCommand::Pause,
        DaemonCommand::Resume,
        DaemonCommand::Yank,
    ];
    for cmd in &variants {
        let s = cmd.to_string();
        let parsed: DaemonCommand = s.parse().unwrap_or_else(|e| {
            panic!("failed to parse {s:?} back to DaemonCommand: {e}");
        });
        assert_eq!(*cmd, parsed, "round-trip failed for {s:?}");
    }
}

/// DaemonCommand display values are the expected lowercase strings.
#[test]
fn daemon_command_display_values() {
    assert_eq!(DaemonCommand::Stop.to_string(), "stop");
    assert_eq!(DaemonCommand::Flush.to_string(), "flush");
    assert_eq!(DaemonCommand::Pause.to_string(), "pause");
    assert_eq!(DaemonCommand::Resume.to_string(), "resume");
    assert_eq!(DaemonCommand::Yank.to_string(), "yank");
}

/// Parsing an unknown string as DaemonCommand returns an error.
#[test]
fn daemon_command_unknown_string_is_error() {
    let result = "bogus".parse::<DaemonCommand>();
    assert!(result.is_err(), "unknown command should fail to parse");
}

// -- DaemonStatus round-trip tests --

/// Every DaemonStatus variant round-trips through Display and FromStr.
#[test]
fn daemon_status_round_trip_all_variants() {
    let variants = [
        DaemonStatus::Recording,
        DaemonStatus::Idle,
        DaemonStatus::Paused,
    ];
    for status in &variants {
        let s = status.to_string();
        let parsed: DaemonStatus = s.parse().unwrap_or_else(|e| {
            panic!("failed to parse {s:?} back to DaemonStatus: {e}");
        });
        assert_eq!(*status, parsed, "round-trip failed for {s:?}");
    }
}

/// DaemonStatus display values are the expected lowercase strings.
#[test]
fn daemon_status_display_values() {
    assert_eq!(DaemonStatus::Recording.to_string(), "recording");
    assert_eq!(DaemonStatus::Idle.to_string(), "idle");
    assert_eq!(DaemonStatus::Paused.to_string(), "paused");
}

/// Parsing an unknown string as DaemonStatus returns an error.
#[test]
fn daemon_status_unknown_string_is_error() {
    let result = "bogus".parse::<DaemonStatus>();
    assert!(result.is_err(), "unknown status should fail to parse");
}

// -- check_command tests --

/// write_fake_lock creates a fake daemon lock file for tests.
fn write_fake_lock(pid: impl std::fmt::Display) {
    let lock = super::super::record_lock_path();
    std::fs::create_dir_all(lock.parent().unwrap()).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(&lock, format!("{pid}:{now}")).unwrap();
}

/// When no command file exists, check_command returns None.
#[test]
fn check_command_no_file_returns_none() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    // Ensure no file exists.
    let _ = fs::remove_file(&cmd_path);

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::None),
        "expected None when no command file exists"
    );
}

/// An unknown command string is logged and treated as None.
#[test]
fn check_command_unknown_string_returns_none() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
    fs::write(&cmd_path, "bogus").unwrap();

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::None),
        "expected None for unknown command"
    );
    // Command file should be removed even on parse error.
    assert!(
        !cmd_path.exists(),
        "command file should be removed after parse error"
    );
}

/// Stop command returns Exit.
#[test]
fn check_command_stop_returns_exit() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
    fs::write(&cmd_path, "stop").unwrap();

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::Exit(DaemonCommand::Stop)),
        "expected Exit(Stop) for stop command"
    );
    assert!(
        !cmd_path.exists(),
        "command file should be removed after processing"
    );
}

/// Yank command returns Exit.
#[test]
fn check_command_yank_returns_exit() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
    fs::write(&cmd_path, "yank").unwrap();

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::Exit(DaemonCommand::Yank)),
        "expected Exit(Yank) for yank command"
    );
}

/// Flush command returns Continue.
#[test]
fn check_command_flush_returns_continue() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
    fs::write(&cmd_path, "flush").unwrap();

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::Continue(DaemonCommand::Flush)),
        "expected Continue(Flush) for flush command"
    );
}

/// Pause command returns Continue.
#[test]
fn check_command_pause_returns_continue() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
    fs::write(&cmd_path, "pause").unwrap();

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::Continue(DaemonCommand::Pause)),
        "expected Continue(Pause) for pause command"
    );
}

/// Resume command returns Continue.
#[test]
fn check_command_resume_returns_continue() {
    let _g = CacheDirGuard::new();
    let cmd_path = super::super::command_path();
    fs::create_dir_all(cmd_path.parent().unwrap()).unwrap();
    fs::write(&cmd_path, "resume").unwrap();

    let result = check_command(&cmd_path);
    assert!(
        matches!(result, CommandResult::Continue(DaemonCommand::Resume)),
        "expected Continue(Resume) for resume command"
    );
}

// -- write_status tests --

/// write_status writes the correct status string to the status file.
#[test]
fn write_status_reflects_recording() {
    let _g = CacheDirGuard::new();
    let status_p = super::super::status_path();
    fs::create_dir_all(status_p.parent().unwrap()).unwrap();

    write_status(DaemonStatus::Recording);

    let content = fs::read_to_string(&status_p).unwrap();
    assert_eq!(content.trim(), "recording");
}

/// After writing Paused status, reading the file yields "paused".
#[test]
fn write_status_reflects_paused() {
    let _g = CacheDirGuard::new();
    let status_p = super::super::status_path();
    fs::create_dir_all(status_p.parent().unwrap()).unwrap();

    write_status(DaemonStatus::Paused);

    let content = fs::read_to_string(&status_p).unwrap();
    assert_eq!(content.trim(), "paused");
}

/// After writing Idle status, reading the file yields "idle".
#[test]
fn write_status_reflects_idle() {
    let _g = CacheDirGuard::new();
    let status_p = super::super::status_path();
    fs::create_dir_all(status_p.parent().unwrap()).unwrap();

    write_status(DaemonStatus::Idle);

    let content = fs::read_to_string(&status_p).unwrap();
    assert_eq!(content.trim(), "idle");
}

// -- CLI pause tests (updated for command/status protocol) --

/// `super::pause()` when not recording prints an error and is a no-op.
#[test]
fn pause_not_recording_is_noop() {
    let _g = CacheDirGuard::new();
    super::pause().unwrap();
    // No command file should exist.
    assert!(!super::super::command_path().exists());
}

/// `super::pause()` sends "pause" when status is "recording",
/// and "resume" when status is "paused".
#[test]
fn pause_toggle_via_status() {
    let _g = CacheDirGuard::new();

    // Simulate a running daemon with recording status.
    write_fake_lock(std::process::id());
    let status_p = super::super::status_path();
    fs::create_dir_all(status_p.parent().unwrap()).unwrap();
    crate::util::atomic_write_str(&status_p, "recording").unwrap();

    // First call: should write "pause" command.
    super::pause().unwrap();
    let cmd = fs::read_to_string(super::super::command_path()).unwrap();
    assert_eq!(cmd.trim(), "pause");

    // Clean up command file (daemon would do this).
    let _ = fs::remove_file(super::super::command_path());

    // Now set status to paused.
    crate::util::atomic_write_str(&status_p, "paused").unwrap();

    // Second call: should write "resume" command.
    super::pause().unwrap();
    let cmd = fs::read_to_string(super::super::command_path()).unwrap();
    assert_eq!(cmd.trim(), "resume");
}
