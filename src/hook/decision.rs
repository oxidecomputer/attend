use std::fs;

use crate::state::SessionId;

use super::types::{GuidanceReason, HookDecision};

/// Pure decision logic for narration delivery hooks (Stop, PreToolUse, PostToolUse).
///
/// Takes all external state as parameters so it can be tested without I/O.
///
/// `stop_hook_active` is set by Claude Code on re-invocation after a previous
/// block. We use it as a safety valve: if we already told the agent to start
/// a receiver and it's re-stopping, approve rather than risk an infinite
/// block loop (e.g. if the receiver hasn't created its lock file yet).
pub(super) fn hook_decision(
    hook_session_id: Option<&SessionId>,
    listening_session: Option<&SessionId>,
    pending_content: Option<String>,
    receiver_alive: bool,
    stop_hook_active: bool,
) -> HookDecision {
    match (listening_session, hook_session_id) {
        // We are the active listening session — check for narration.
        (Some(listening_sid), Some(hook_sid)) if listening_sid == hook_sid => {}
        // Narration is active in a different session.
        (Some(_), Some(_)) => return HookDecision::block(GuidanceReason::SessionMoved),
        // No listening session at all — approve silently.
        _ => return HookDecision::Silent,
    }

    // We are the active session. Pending narration always takes priority —
    // deliver it regardless of stop_hook_active.
    if let Some(content) = pending_content {
        return HookDecision::PendingNarration {
            content,
            effect: super::types::GuidanceEffect::Block,
        };
    }

    // No narration. If a receiver is running, it will handle future delivery.
    if receiver_alive {
        return HookDecision::Silent;
    }

    // No receiver. On re-invocation after a previous block, approve to avoid
    // an infinite loop (the agent already got the "start receiver" message).
    if stop_hook_active {
        return HookDecision::Silent;
    }

    // First attempt, no receiver — ask the agent to start one.
    HookDecision::block(GuidanceReason::StartReceiver)
}

/// Check whether a background `receive --wait` process is alive.
pub(super) fn receiver_alive() -> bool {
    let lock_path = crate::narrate::receive_lock_path();
    let Ok(content) = fs::read_to_string(&lock_path) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<i32>() else {
        return false;
    };
    crate::narrate::process_alive(pid)
}
