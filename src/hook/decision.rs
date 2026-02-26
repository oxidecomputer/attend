use std::fs;

use super::types::{GuidanceReason, HookDecision, HookType};

/// Relationship between the hook's session and the active listening session.
///
/// Computed once by the dispatcher and passed to decision functions so they
/// don't need raw session IDs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SessionRelation {
    /// This session is the active listener.
    Active,
    /// Another session owns narration (this session was displaced).
    Stolen,
    /// No listening session exists, or no session ID on this hook.
    Inactive,
}

/// Pure decision logic for general (non-listen) hooks: Stop, PreToolUse,
/// PostToolUse on tools other than `attend listen`.
///
/// Returns final decisions with the correct effect — callers should not
/// transform Block↔Approve.
///
/// `stop_hook_active` is set by Claude Code on re-invocation after a previous
/// block. We use it as a safety valve: if we already told the agent to start
/// a receiver and it's re-stopping, approve rather than risk an infinite
/// block loop (e.g. if the receiver hasn't created its lock file yet).
pub(super) fn general_decision(
    relation: SessionRelation,
    has_pending: bool,
    receiver_alive: bool,
    stop_hook_active: bool,
    hook_type: HookType,
) -> HookDecision {
    match relation {
        SessionRelation::Inactive => HookDecision::Silent,

        // Advisory: inform the agent that narration was displaced.
        SessionRelation::Stolen => HookDecision::approve(GuidanceReason::SessionMoved),

        SessionRelation::Active => {
            // Pending narration always takes priority: block so the agent
            // runs `attend listen` to pick up the content first.
            if has_pending {
                return HookDecision::block(GuidanceReason::NarrationReady);
            }

            // A running receiver will handle future delivery.
            if receiver_alive {
                return HookDecision::Silent;
            }

            // Re-invocation after a previous block: approve to avoid an
            // infinite loop (the agent already got the "start receiver"
            // message).
            if stop_hook_active {
                return HookDecision::Silent;
            }

            // First attempt, no receiver — nudge the agent to start one.
            // Block on Stop (don't exit before starting), advisory on ToolUse
            // (let the tool through).
            match hook_type {
                HookType::Stop => HookDecision::block(GuidanceReason::StartReceiver),
                _ => HookDecision::approve(GuidanceReason::StartReceiver),
            }
        }
    }
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
