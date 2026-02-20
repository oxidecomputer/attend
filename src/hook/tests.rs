use super::*;
use decision::SessionRelation;

// ---------------------------------------------------------------------------
// Exhaustive enumeration of the decision space
//
// general_decision has 5 inputs: relation (3) × has_pending (2) ×
// receiver_alive (2) × stop_hook_active (2) × hook_type (3) = 72
// combinations. Small enough to enumerate exhaustively, giving complete
// coverage with no randomness.
// ---------------------------------------------------------------------------

const ALL_RELATIONS: [SessionRelation; 3] = [
    SessionRelation::Active,
    SessionRelation::Stolen,
    SessionRelation::Inactive,
];

/// Only the hook types that reach `general_decision`. SessionStart and
/// UserPrompt are handled by separate code paths.
const ALL_HOOK_TYPES: [HookType; 3] = [HookType::Stop, HookType::PreToolUse, HookType::PostToolUse];

const ALL_BOOLS: [bool; 2] = [false, true];

/// Invoke `f` for every one of the 72 input combinations.
fn for_all(mut f: impl FnMut(SessionRelation, bool, bool, bool, HookType)) {
    for &relation in &ALL_RELATIONS {
        for &has_pending in &ALL_BOOLS {
            for &receiver_alive in &ALL_BOOLS {
                for &stop_hook_active in &ALL_BOOLS {
                    for &hook_type in &ALL_HOOK_TYPES {
                        f(
                            relation,
                            has_pending,
                            receiver_alive,
                            stop_hook_active,
                            hook_type,
                        );
                    }
                }
            }
        }
    }
}

/// Decision helpers for readable assertions.
fn is_block(d: &HookDecision) -> bool {
    matches!(
        d,
        HookDecision::Guidance {
            effect: GuidanceEffect::Block,
            ..
        }
    )
}

fn effect_of(d: &HookDecision) -> Option<GuidanceEffect> {
    match d {
        HookDecision::Silent => None,
        HookDecision::Guidance { effect, .. } => Some(*effect),
    }
}

fn reason_of(d: &HookDecision) -> Option<&GuidanceReason> {
    match d {
        HookDecision::Silent => None,
        HookDecision::Guidance { reason, .. } => Some(reason),
    }
}

// ---------------------------------------------------------------------------
// Invariant tests — each checks one property across all 72 combinations
//
// Documented with:
//  - the invariant being verified
//  - the concurrent flow / failure mode it guards against
// ---------------------------------------------------------------------------

/// **Invariant**: Inactive and Stolen sessions never receive a Block.
///
/// **Flow**: Multiple Claude Code sessions are open. Some have never run
/// `/attend` (Inactive). Others had narration stolen away (Stolen).
/// Neither should have tool calls or stop hooks blocked — that would
/// disrupt unrelated work in sessions that aren't participating in
/// (or have been displaced from) narration.
#[test]
fn non_active_sessions_never_blocked() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation == SessionRelation::Active {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert!(
                !is_block(&d),
                "non-active session got Block: relation={relation:?}, \
             has_pending={has_pending}, receiver_alive={receiver_alive}, \
             stop_hook_active={stop_hook_active}, hook_type={hook_type:?}, \
             decision={d:?}"
            );
        },
    );
}

/// **Invariant**: SessionMoved is never delivered as a Block.
///
/// **Flow**: Session A was the active listener. Session B runs `/attend`,
/// stealing narration. Session A's next hook fires. The SessionMoved
/// notification tells A "narration moved away" but must not block A's
/// tools — A might be mid-task, writing files, running tests. Blocking
/// would disrupt that work for no benefit.
#[test]
fn session_moved_is_never_block() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            if reason_of(&d) == Some(&GuidanceReason::SessionMoved) {
                assert_eq!(
                    effect_of(&d),
                    Some(GuidanceEffect::Approve),
                    "SessionMoved should be Approve: relation={relation:?}, hook_type={hook_type:?}"
                );
            }
        },
    );
}

/// **Invariant**: Inactive sessions always produce Silent — no output at
/// all, regardless of other state.
///
/// **Flow**: A session that never activated `/attend` fires a hook while
/// some other session is listening, or while no session is listening, or
/// while a receiver is alive somewhere. None of that matters: this
/// session is not a narration participant and should be completely
/// unaware of the attend system.
#[test]
fn inactive_always_silent() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation != SessionRelation::Inactive {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::Silent,
                "Inactive should always be Silent: has_pending={has_pending}, \
             receiver_alive={receiver_alive}, hook_type={hook_type:?}"
            );
        },
    );
}

/// **Invariant**: A stolen session's decision is independent of
/// `has_pending`, `receiver_alive`, `stop_hook_active`, and `hook_type`.
///
/// **Flow**: Session A was displaced. Meanwhile narration is piling up,
/// the receiver crashed, or the stop hook is re-firing. None of that
/// is A's concern anymore — it has been displaced. The decision should
/// be a fixed advisory regardless of what's happening in the narration
/// subsystem. Checking narration state for a stolen session would be
/// a bug: it could cause a displaced session to start a receiver or
/// attempt delivery for content it no longer owns.
#[test]
fn stolen_decision_ignores_other_state() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation != SessionRelation::Stolen {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::approve(GuidanceReason::SessionMoved),
                "Stolen should always be Approve(SessionMoved): has_pending={has_pending}, \
             receiver_alive={receiver_alive}, hook_type={hook_type:?}"
            );
        },
    );
}

/// **Invariant**: When the active session has pending narration, the
/// decision is Block(NarrationReady) regardless of receiver state,
/// stop_hook_active, or hook type.
///
/// **Flow**: The user is narrating, and events have been written to the
/// pending directory. The agent is mid-response, making tool calls. If
/// there's pending narration, we *must* block: this forces the agent to
/// run `attend listen` to pick up the content before continuing. If we
/// didn't block, narration could go stale indefinitely while the agent
/// keeps working.
///
/// The "regardless of receiver state" part matters because of a race:
/// the receiver might be technically alive but hasn't consumed the
/// pending files yet. Or the receiver might have crashed right after
/// files appeared. Either way, pending files = block.
#[test]
fn pending_narration_always_blocks() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            if relation != SessionRelation::Active || !has_pending {
                return;
            }
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::block(GuidanceReason::NarrationReady),
                "Active + pending should always Block(NarrationReady): \
             receiver_alive={receiver_alive}, stop_hook_active={stop_hook_active}, \
             hook_type={hook_type:?}"
            );
        },
    );
}

/// **Invariant**: When the active session has no pending narration, no
/// receiver, and this is the first attempt (not a re-invocation), the
/// decision always carries `StartReceiver`.
///
/// **Flow**: The receiver process crashed (or was never started). The
/// agent doesn't know yet. Without a receiver, future narration will
/// pile up in the pending directory with no delivery mechanism. The hook
/// must notice the gap and tell the agent to start one. If it doesn't,
/// the user's narration goes undelivered until something else triggers
/// receiver startup.
#[test]
fn missing_receiver_detected() {
    for &hook_type in &ALL_HOOK_TYPES {
        let d = general_decision(
            SessionRelation::Active,
            false, // no pending
            false, // no receiver
            false, // first attempt
            hook_type,
        );
        assert_eq!(
            reason_of(&d),
            Some(&GuidanceReason::StartReceiver),
            "missing receiver should produce StartReceiver: hook_type={hook_type:?}"
        );
    }
}

/// **Invariant**: When stop_hook_active is true and there's no pending
/// narration, the decision is Silent regardless of receiver state.
///
/// **Flow**: The Stop hook fired, returned Block(StartReceiver), and
/// Claude Code re-invoked the hook with stop_hook_active=true. If we
/// block again, we get an infinite loop: block -> re-invoke -> block ->
/// re-invoke. The safety valve MUST release. This is especially
/// important in the race where the agent started the receiver but its
/// lock file hasn't been written yet: receiver_alive is false but
/// blocking again would be wrong.
///
/// The only exception is pending narration (tested separately in
/// `pending_narration_always_blocks`): if narration arrived during
/// the re-invocation cycle, we block with NarrationReady, not
/// StartReceiver. That's safe because NarrationReady is a different
/// action (run `attend listen`) that breaks the StartReceiver loop.
#[test]
fn reentry_safety_valve_releases() {
    for &receiver_alive in &ALL_BOOLS {
        for &hook_type in &ALL_HOOK_TYPES {
            let d = general_decision(
                SessionRelation::Active,
                false, // no pending
                receiver_alive,
                true, // stop_hook_active: re-invocation
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::Silent,
                "re-invocation should release to Silent: receiver_alive={receiver_alive}, \
                 hook_type={hook_type:?}"
            );
        }
    }
}

/// **Invariant**: StartReceiver uses Block on Stop hooks but Approve on
/// ToolUse hooks.
///
/// **Flow**: The receiver is dead and needs restarting. Two scenarios:
///
/// 1. **Stop hook**: The agent is about to exit. If we approve, the
///    session exits with no receiver, and future narration goes
///    undelivered. We MUST block to give the agent a chance to start
///    the receiver before exiting.
///
/// 2. **PreToolUse/PostToolUse**: The agent is executing a tool (e.g.
///    reading a file, running a test). Blocking that tool call just
///    because the receiver is down is too disruptive. Instead, approve
///    the tool but inject an advisory nudge.
#[test]
fn start_receiver_effect_by_hook_type() {
    for &hook_type in &ALL_HOOK_TYPES {
        let d = general_decision(
            SessionRelation::Active,
            false, // no pending
            false, // no receiver
            false, // first attempt
            hook_type,
        );
        let expected = match hook_type {
            HookType::Stop => GuidanceEffect::Block,
            _ => GuidanceEffect::Approve,
        };
        assert_eq!(
            effect_of(&d),
            Some(expected),
            "StartReceiver effect wrong for hook_type={hook_type:?}"
        );
    }
}

/// **Invariant**: NarrationReady is always a Block, regardless of hook
/// type.
///
/// **Flow**: Narration has arrived and is sitting in pending files.
/// Whether this is a Stop hook (agent exiting), PreToolUse (about to
/// run a tool), or PostToolUse (tool just ran), the agent must pick up
/// the narration before continuing. This is the synchronous delivery
/// trigger: the agent runs `attend listen`, its PreToolUse hook delivers
/// the content and starts a new receiver in one round trip.
///
/// If we approved instead of blocking, the agent would continue without
/// the narration content, and the user's spoken context would go
/// undelivered.
#[test]
fn narration_ready_always_blocks() {
    for_all(
        |relation, has_pending, receiver_alive, stop_hook_active, hook_type| {
            let d = general_decision(
                relation,
                has_pending,
                receiver_alive,
                stop_hook_active,
                hook_type,
            );
            if reason_of(&d) == Some(&GuidanceReason::NarrationReady) {
                assert!(
                    is_block(&d),
                    "NarrationReady should always block: relation={relation:?}, \
                 hook_type={hook_type:?}, decision={d:?}"
                );
            }
        },
    );
}

/// **Invariant**: When the active session has no pending narration and a
/// receiver is alive, the decision is Silent regardless of other flags.
///
/// **Flow**: Everything is working normally. The receiver is running in
/// the background, polling for new narration events. No events have
/// arrived yet (no pending). The hook should be completely transparent:
/// the receiver will handle delivery when narration arrives. Any
/// non-Silent output here would be noise.
#[test]
fn receiver_alive_no_pending_is_silent() {
    for &stop_hook_active in &ALL_BOOLS {
        for &hook_type in &ALL_HOOK_TYPES {
            let d = general_decision(
                SessionRelation::Active,
                false, // no pending
                true,  // receiver alive
                stop_hook_active,
                hook_type,
            );
            assert_eq!(
                d,
                HookDecision::Silent,
                "receiver alive + no pending should be Silent: \
                 stop_hook_active={stop_hook_active}, hook_type={hook_type:?}"
            );
        }
    }
}

// --- general_decision point tests ---

/// Inactive session (no listening session or no session ID): silent.
#[test]
fn general_inactive_silent() {
    let d = general_decision(
        SessionRelation::Inactive,
        false,
        false,
        false,
        HookType::Stop,
    );
    assert_eq!(d, HookDecision::Silent);
}

/// Stolen session: advisory SessionMoved (approve, not block).
#[test]
fn general_stolen_session_moved() {
    let d = general_decision(SessionRelation::Stolen, false, false, false, HookType::Stop);
    assert_eq!(d, HookDecision::approve(GuidanceReason::SessionMoved));
}

/// Active session with pending narration: block with NarrationReady.
#[test]
fn general_active_pending_narration() {
    let d = general_decision(SessionRelation::Active, true, false, false, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Pending narration takes priority over a running receiver.
#[test]
fn general_pending_takes_priority_over_receiver() {
    let d = general_decision(SessionRelation::Active, true, true, false, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Pending narration takes priority even on re-invocation.
#[test]
fn general_pending_takes_priority_over_reentry() {
    let d = general_decision(SessionRelation::Active, true, false, true, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::NarrationReady));
}

/// Receiver alive, no pending: silent.
#[test]
fn general_active_receiver_alive_no_pending() {
    let d = general_decision(SessionRelation::Active, false, true, false, HookType::Stop);
    assert_eq!(d, HookDecision::Silent);
}

/// No receiver, no pending, first attempt on Stop: block to start receiver.
#[test]
fn general_stop_no_receiver_blocks() {
    let d = general_decision(SessionRelation::Active, false, false, false, HookType::Stop);
    assert_eq!(d, HookDecision::block(GuidanceReason::StartReceiver));
}

/// No receiver, no pending, first attempt on PreToolUse: advisory to start receiver.
#[test]
fn general_pre_tool_use_no_receiver_approves() {
    let d = general_decision(
        SessionRelation::Active,
        false,
        false,
        false,
        HookType::PreToolUse,
    );
    assert_eq!(d, HookDecision::approve(GuidanceReason::StartReceiver));
}

/// Re-invocation after a previous block, no receiver: silent to avoid loop.
#[test]
fn general_active_reentry_no_receiver_silent() {
    let d = general_decision(SessionRelation::Active, false, false, true, HookType::Stop);
    assert_eq!(d, HookDecision::Silent);
}

/// Re-invocation with receiver alive: silent.
#[test]
fn general_active_reentry_receiver_alive_silent() {
    let d = general_decision(SessionRelation::Active, false, true, true, HookType::Stop);
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
