use crate::hook::{GuidanceEffect, GuidanceReason, HookDecision, HookInput, HookType};
use crate::state::{EditorState, SessionId};

/// Deliver narration content and approve the `attend listen` tool call.
///
/// This is the sole content delivery path: `attend listen`'s PreToolUse
/// reads pending files, formats them, and calls this to emit the content
/// with an "allow" so the listener starts in the same round trip.
pub(super) fn deliver_narration(content: &str) -> anyhow::Result<()> {
    let narration = format!(
        "<narration>\n{content}\n</narration>\n\
         <system-instruction>\n\
         {}\n\
         </system-instruction>",
        include_str!("../messages/narration_pause.txt")
    );
    let response = serde_json::json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "allow",
            "additionalContext": narration
        }
    });
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

/// Emit session-start output: instructions + optional narration re-emission.
pub(super) fn session_start(_input: &HookInput, is_listening: bool) -> anyhow::Result<()> {
    let bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "attend".to_string());

    // Emit instructions (templated with the binary path)
    println!("<editor-context-instructions>");
    print!(
        include_str!("../messages/editor_context_instructions.txt"),
        bin_cmd = bin
    );
    println!("</editor-context-instructions>");

    // If this session is actively listening for narration, re-emit the
    // narration skill instructions so the agent restarts its background
    // receiver after context compaction or clear.
    if is_listening {
        print!("{}", narration_instructions(&bin));
    }

    Ok(())
}

/// Emit editor context when state has changed.
pub(super) fn editor_context(state: &EditorState) -> anyhow::Result<()> {
    println!("<editor-context>\n{state}\n</editor-context>");
    Ok(())
}

/// Emit /attend activation response.
pub(super) fn attend_activate(_session_id: &SessionId) -> anyhow::Result<()> {
    let response = serde_json::json!({
        "additionalContext": include_str!("../messages/activate_response.txt")
    });
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

/// Emit /unattend deactivation response.
pub(super) fn attend_deactivate(_session_id: &SessionId) -> anyhow::Result<()> {
    let response = serde_json::json!({
        "additionalContext": include_str!("../messages/deactivate_response.txt")
    });
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

/// Emit hook decision as JSON for Claude Code.
///
/// Output format varies by hook type:
/// - **PreToolUse**: `hookSpecificOutput` with `permissionDecision` (allow/deny)
///   and `additionalContext` for content that Claude should see.
/// - **PostToolUse**: top-level `decision`/`reason` with
///   `hookSpecificOutput.additionalContext` for supplementary context.
/// - **Stop**: top-level `decision: "block"` with `reason` (shown to Claude).
pub(super) fn attend_result(decision: &HookDecision, hook_type: HookType) -> anyhow::Result<()> {
    match decision {
        HookDecision::Silent => {}
        HookDecision::Guidance { reason, effect } => {
            let message = guidance_message(reason);
            let response = render_decision(hook_type, effect, message);
            println!("{}", serde_json::to_string(&response)?);
        }
    }
    Ok(())
}

/// Render a hook decision in the format appropriate for the hook type.
fn render_decision(
    hook_type: HookType,
    effect: &GuidanceEffect,
    message: &str,
) -> serde_json::Value {
    // Wrap in <system-instruction> tags for fields that pass through as
    // context to Claude (additionalContext, permissionDecisionReason).
    // Stop hook's `reason` is already surfaced directly, so no wrapping.
    let wrapped = format!("<system-instruction>\n{message}\n</system-instruction>");

    match hook_type {
        // PreToolUse: hookSpecificOutput with permissionDecision.
        // additionalContext reaches Claude; permissionDecisionReason
        // reaches Claude on deny, user-only on allow.
        HookType::PreToolUse => {
            let decision = match effect {
                GuidanceEffect::Block => "deny",
                GuidanceEffect::Approve => "allow",
            };
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": decision,
                    "additionalContext": wrapped
                }
            })
        }
        // PostToolUse: top-level decision + hookSpecificOutput.additionalContext.
        HookType::PostToolUse => match effect {
            GuidanceEffect::Block => serde_json::json!({
                "decision": "block",
                "reason": wrapped
            }),
            GuidanceEffect::Approve => serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "PostToolUse",
                    "additionalContext": wrapped
                }
            }),
        },
        // Stop: top-level decision/reason. The `reason` field is shown
        // directly to both Claude and the user — no XML wrapping.
        HookType::Stop => match effect {
            GuidanceEffect::Block => serde_json::json!({
                "decision": "block",
                "reason": message
            }),
            GuidanceEffect::Approve => serde_json::json!({}),
        },
        // SessionStart, UserPrompt, SessionEnd don't use attend_result.
        _ => serde_json::json!({}),
    }
}

/// Map a guidance reason to an agent-facing message string.
fn guidance_message(reason: &GuidanceReason) -> &'static str {
    match reason {
        GuidanceReason::SessionMoved => include_str!("../messages/guidance_session_moved.txt"),
        GuidanceReason::StartReceiver => include_str!("../messages/guidance_start_receiver.txt"),
        GuidanceReason::NarrationReady => {
            include_str!("../messages/guidance_narration_ready.txt")
        }
        GuidanceReason::ListenerAlreadyActive => {
            include_str!("../messages/guidance_listener_active.txt")
        }
        GuidanceReason::ListenerStarted => {
            include_str!("../messages/guidance_listener_started.txt")
        }
        GuidanceReason::Deactivated => {
            include_str!("../messages/guidance_deactivated.txt")
        }
    }
}

/// Build narration skill instructions for re-emission after context compaction.
///
/// Uses `skill_body.md` — the same body as the installed SKILL.md,
/// so the instructions stay consistent with the skill template.
fn narration_instructions(bin_cmd: &str) -> String {
    let protocol = include_str!("../messages/narration_protocol.md");
    let body = format!(
        include_str!("messages/skill_body.md"),
        bin_cmd = bin_cmd,
        narration_protocol = protocol,
    );
    format!("\n<narration-instructions>\n{body}</narration-instructions>\n")
}
