use crate::state::SessionId;

use super::*;

/// Helper to build a `SessionId` from a literal.
fn sid(s: &str) -> SessionId {
    SessionId::from(s)
}

// --- hook_decision tests ---

/// No listening session at all: approve silently.
#[test]
fn stop_no_listening_session() {
    let hook = sid("abc");
    let d = hook_decision(Some(&hook), None, false, false, false);
    assert_eq!(d, HookDecision::Silent);
}

/// No hook session ID: approve silently.
#[test]
fn stop_no_hook_session_id() {
    let listening = sid("abc");
    let d = hook_decision(None, Some(&listening), false, false, false);
    assert_eq!(d, HookDecision::Silent);
}

/// Neither session present: approve silently.
#[test]
fn stop_neither_session() {
    let d = hook_decision(None, None, false, false, false);
    assert_eq!(d, HookDecision::Silent);
}

/// Narration is active in a different session: deliver guidance once.
#[test]
fn stop_session_moved() {
    let hook = sid("mine");
    let listening = sid("other");
    let d = hook_decision(Some(&hook), Some(&listening), false, false, false);
    assert_eq!(d, HookDecision::block(GuidanceReason::SessionMoved));
}

/// Active session with pending narration: block with NarrationReady.
#[test]
fn decision_active_pending_narration() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), true, false, false);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Pending narration takes priority over a running receiver.
#[test]
fn decision_pending_takes_priority_over_receiver() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), true, true, false);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Pending narration takes priority even on re-invocation.
#[test]
fn decision_pending_takes_priority_over_reentry() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), true, false, true);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Receiver alive, no pending: approve silently.
#[test]
fn stop_active_receiver_alive_no_pending() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), false, true, false);
    assert_eq!(d, HookDecision::Silent);
}

/// No receiver, no pending, first attempt: ask agent to start receiver.
#[test]
fn stop_active_no_receiver_no_pending() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), false, false, false);
    assert_eq!(d, HookDecision::block(GuidanceReason::StartReceiver));
}

/// Re-invocation after a previous block, no receiver: approve to avoid loop.
#[test]
fn stop_active_reentry_no_receiver_approves() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), false, false, true);
    assert_eq!(d, HookDecision::Silent);
}

/// Re-invocation with receiver alive: approve silently.
#[test]
fn stop_active_reentry_receiver_alive_approves() {
    let s = sid("abc");
    let d = hook_decision(Some(&s), Some(&s), false, true, true);
    assert_eq!(d, HookDecision::Silent);
}

// --- is_attend_prompt tests ---

/// Exact `/attend` match.
#[test]
fn is_attend_prompt_exact() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("/attend".into()),
        },
        ..Default::default()
    };
    assert!(is_attend_prompt(&input));
}

/// `/attend` with surrounding whitespace.
#[test]
fn is_attend_prompt_with_whitespace() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("  /attend  ".into()),
        },
        ..Default::default()
    };
    assert!(is_attend_prompt(&input));
}

/// Non-attend prompt text.
#[test]
fn is_attend_prompt_different_text() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("hello world".into()),
        },
        ..Default::default()
    };
    assert!(!is_attend_prompt(&input));
}

/// No prompt field at all.
#[test]
fn is_attend_prompt_no_prompt_field() {
    let input = HookInput::default();
    assert!(!is_attend_prompt(&input));
}

/// Partial match: `/attend to this` should not match.
#[test]
fn is_attend_prompt_partial() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("/attend to this".into()),
        },
        ..Default::default()
    };
    assert!(!is_attend_prompt(&input));
}

// --- is_listen_command tests ---

/// Bare binary name matches.
#[test]
fn listen_command_bare_name() {
    assert!(is_listen_command("attend listen", "attend"));
}

/// Full path matches against filename component.
#[test]
fn listen_command_full_path() {
    assert!(is_listen_command("/usr/local/bin/attend listen", "attend"));
}

/// Extra flags after `listen` are allowed.
#[test]
fn listen_command_with_flags() {
    assert!(is_listen_command("attend listen --check", "attend"));
}

/// Different subcommand is not matched.
#[test]
fn listen_command_different_subcommand() {
    assert!(!is_listen_command("attend narrate status", "attend"));
}

/// Different binary name is not matched.
#[test]
fn listen_command_different_binary() {
    assert!(!is_listen_command("cargo test", "attend"));
}

/// Empty command is not matched.
#[test]
fn listen_command_empty() {
    assert!(!is_listen_command("", "attend"));
}

/// Binary-only (no subcommand) is not matched.
#[test]
fn listen_command_no_subcommand() {
    assert!(!is_listen_command("attend", "attend"));
}
