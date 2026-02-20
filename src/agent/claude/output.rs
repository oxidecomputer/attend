use crate::hook::{GuidanceEffect, GuidanceReason, HookDecision, HookInput, HookType};
use crate::state::{EditorState, SessionId};

/// Emit session-start output: instructions + optional narration re-emission.
pub(super) fn session_start(_input: &HookInput, is_listening: bool) -> anyhow::Result<()> {
    let bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "attend".to_string());

    // Emit instructions (templated with the binary path)
    println!("<editor-context-instructions>");
    print!(
        include_str!("messages/editor_context_instructions.txt"),
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
        "additionalContext": include_str!("messages/activate_response.txt")
    });
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

/// Emit hook decision as JSON for Claude Code.
pub(super) fn attend_result(decision: &HookDecision, hook_type: HookType) -> anyhow::Result<()> {
    match decision {
        HookDecision::Silent => {}
        HookDecision::PendingNarration { content } => {
            let reason = if matches!(hook_type, HookType::ToolUse) {
                format!(
                    "{content}\n\
                     <system-instruction>\n\
                     {}\n\
                     </system-instruction>",
                    include_str!("messages/narration_pause.txt")
                )
            } else {
                content.clone()
            };
            let response = serde_json::json!({
                "decision": "block",
                "reason": reason
            });
            println!("{}", serde_json::to_string(&response)?);
        }
        HookDecision::Guidance { reason, effect } => {
            let action = match effect {
                GuidanceEffect::Block => "block",
                GuidanceEffect::Approve => "approve",
            };
            let message = guidance_message(reason);
            let response = serde_json::json!({
                "decision": action,
                "reason": message
            });
            println!("{}", serde_json::to_string(&response)?);
        }
    }
    Ok(())
}

/// Map a guidance reason to an agent-facing message string.
fn guidance_message(reason: &GuidanceReason) -> &'static str {
    match reason {
        GuidanceReason::SessionMoved => include_str!("messages/guidance_session_moved.txt"),
        GuidanceReason::StartReceiver => include_str!("messages/guidance_start_receiver.txt"),
        GuidanceReason::ListenerAlreadyActive => {
            include_str!("messages/guidance_listener_active.txt")
        }
    }
}

/// Build narration skill instructions for re-emission after context compaction.
///
/// Uses `skill_body.md` — the same body as the installed SKILL.md,
/// so the instructions stay consistent with the skill template.
fn narration_instructions(bin_cmd: &str) -> String {
    let body = format!(include_str!("messages/skill_body.md"), bin_cmd = bin_cmd);
    format!("\n<narration-instructions>\n{body}</narration-instructions>\n")
}
