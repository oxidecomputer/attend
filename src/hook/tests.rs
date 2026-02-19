use super::*;

// --- stop_decision tests ---

#[test]
fn stop_no_listening_session() {
    let d = stop_decision(Some("abc"), None, None, false, false);
    assert_eq!(d, StopDecision::Silent);
}

#[test]
fn stop_no_hook_session_id() {
    let d = stop_decision(None, Some("abc"), None, false, false);
    assert_eq!(d, StopDecision::Silent);
}

#[test]
fn stop_neither_session() {
    let d = stop_decision(None, None, None, false, false);
    assert_eq!(d, StopDecision::Silent);
}

#[test]
fn stop_session_moved() {
    let d = stop_decision(Some("mine"), Some("other"), None, false, false);
    assert!(matches!(d, StopDecision::Approve { reason } if reason.contains("moved")));
}

#[test]
fn stop_session_moved_says_no_restart() {
    let d = stop_decision(Some("mine"), Some("other"), None, false, false);
    if let StopDecision::Approve { reason } = d {
        assert!(reason.contains("Do not restart"));
        assert!(reason.contains("/attend"));
    } else {
        panic!("expected Approve, got {d:?}");
    }
}

#[test]
fn stop_active_with_pending_dictation() {
    let d = stop_decision(
        Some("abc"),
        Some("abc"),
        Some("<dictation>hello</dictation>".into()),
        false,
        false,
    );
    assert!(matches!(d, StopDecision::Block { reason } if reason.contains("hello")));
}

#[test]
fn stop_active_pending_takes_priority_over_receiver() {
    let d = stop_decision(
        Some("abc"),
        Some("abc"),
        Some("<dictation>hello</dictation>".into()),
        true,
        false,
    );
    assert!(matches!(d, StopDecision::Block { reason } if reason.contains("hello")));
}

#[test]
fn stop_active_pending_takes_priority_over_reentry() {
    // Even on re-invocation, pending dictation must be delivered.
    let d = stop_decision(
        Some("abc"),
        Some("abc"),
        Some("<dictation>hello</dictation>".into()),
        false,
        true,
    );
    assert!(matches!(d, StopDecision::Block { reason } if reason.contains("hello")));
}

#[test]
fn stop_active_receiver_alive_no_pending() {
    let d = stop_decision(Some("abc"), Some("abc"), None, true, false);
    assert_eq!(d, StopDecision::Silent);
}

#[test]
fn stop_active_no_receiver_no_pending() {
    let d = stop_decision(Some("abc"), Some("abc"), None, false, false);
    assert!(matches!(d, StopDecision::Block { reason } if reason.contains("receive --wait")));
}

#[test]
fn stop_active_reentry_no_receiver_approves() {
    // Re-invocation after a previous block: approve even if receiver isn't
    // alive yet, to avoid an infinite block loop.
    let d = stop_decision(Some("abc"), Some("abc"), None, false, true);
    assert_eq!(d, StopDecision::Silent);
}

#[test]
fn stop_active_reentry_receiver_alive_approves() {
    let d = stop_decision(Some("abc"), Some("abc"), None, true, true);
    assert_eq!(d, StopDecision::Silent);
}

// --- is_attend_prompt tests ---

#[test]
fn is_attend_prompt_exact() {
    let json = serde_json::json!({"prompt": "/attend", "session_id": "abc"});
    assert!(is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_with_whitespace() {
    let json = serde_json::json!({"prompt": "  /attend  ", "session_id": "abc"});
    assert!(is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_different_text() {
    let json = serde_json::json!({"prompt": "hello world", "session_id": "abc"});
    assert!(!is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_no_prompt_field() {
    let json = serde_json::json!({"session_id": "abc"});
    assert!(!is_attend_prompt(&json));
}

#[test]
fn is_attend_prompt_partial() {
    let json = serde_json::json!({"prompt": "/attend to this", "session_id": "abc"});
    assert!(!is_attend_prompt(&json));
}
