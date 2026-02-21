use super::super::*;

// ---------------------------------------------------------------------------
// is_listen_command tests
// ---------------------------------------------------------------------------

/// Bare binary name matches.
#[test]
fn bare_name() {
    assert!(is_listen_command("attend listen", "attend"));
}

/// Full path matches against filename component.
#[test]
fn full_path() {
    assert!(is_listen_command("/usr/local/bin/attend listen", "attend"));
}

/// Extra flags after `listen` are allowed.
#[test]
fn with_flags() {
    assert!(is_listen_command("attend listen --check", "attend"));
}

/// Different subcommand is not matched.
#[test]
fn different_subcommand() {
    assert!(!is_listen_command("attend narrate status", "attend"));
}

/// Different binary name is not matched.
#[test]
fn different_binary() {
    assert!(!is_listen_command("cargo test", "attend"));
}

/// Empty command is not matched.
#[test]
fn empty() {
    assert!(!is_listen_command("", "attend"));
}

/// Binary-only (no subcommand) is not matched.
#[test]
fn no_subcommand() {
    assert!(!is_listen_command("attend", "attend"));
}
