use super::super::*;

// ---------------------------------------------------------------------------
// parse_listen_command tests
// ---------------------------------------------------------------------------

/// Bare binary name matches as Listen.
#[test]
fn bare_name() {
    assert_eq!(
        parse_listen_command("attend listen", "attend"),
        Some(ListenCommand::Listen)
    );
}

/// Full path matches against filename component.
#[test]
fn full_path() {
    assert_eq!(
        parse_listen_command("/usr/local/bin/attend listen", "attend"),
        Some(ListenCommand::Listen)
    );
}

/// Extra flags after `listen` are allowed (non-stop).
#[test]
fn with_flags() {
    assert_eq!(
        parse_listen_command("attend listen --check", "attend"),
        Some(ListenCommand::Listen)
    );
}

/// `--stop` flag is detected as ListenStop.
#[test]
fn with_stop_flag() {
    assert_eq!(
        parse_listen_command("attend listen --stop", "attend"),
        Some(ListenCommand::ListenStop)
    );
}

/// `--stop` with other flags is still ListenStop.
#[test]
fn stop_with_other_flags() {
    assert_eq!(
        parse_listen_command("attend listen --session abc --stop", "attend"),
        Some(ListenCommand::ListenStop)
    );
}

/// Different subcommand is not matched.
#[test]
fn different_subcommand() {
    assert_eq!(
        parse_listen_command("attend narrate status", "attend"),
        None
    );
}

/// Different binary name is not matched.
#[test]
fn different_binary() {
    assert_eq!(parse_listen_command("cargo test", "attend"), None);
}

/// Empty command is not matched.
#[test]
fn empty() {
    assert_eq!(parse_listen_command("", "attend"), None);
}

/// Binary-only (no subcommand) is not matched.
#[test]
fn no_subcommand() {
    assert_eq!(parse_listen_command("attend", "attend"), None);
}
