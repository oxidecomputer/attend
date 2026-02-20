mod command;
mod decision;
mod session_state;
mod types;
mod upgrade;

use std::fs;
use std::io;

use camino::Utf8PathBuf;

use crate::agent::Agent;
use crate::state::{self, EditorState};

// Public API: re-export all domain types.
pub use types::{GuidanceEffect, GuidanceReason, HookDecision, HookInput, HookKind, HookType};

// Crate-internal re-export.
pub(crate) use session_state::clear_session_moved_marker;

// Internal imports from submodules.
#[cfg(test)]
use command::is_listen_command;
use command::{is_attend_listen, is_attend_prompt};
use decision::{hook_decision, receiver_alive};
use session_state::{
    clean_moved_markers, mark_session_moved_notified, session_cache_path,
    session_moved_already_notified,
};
use upgrade::auto_upgrade_hooks;

/// Handle the `SessionStart` hook: clear cache, auto-upgrade, emit instructions.
///
/// On compact/clear, if this session is actively listening for narration,
/// re-emit the narration skill instructions so the agent knows to restart
/// its background receiver.
///
/// Also checks whether the running binary version matches the version that
/// installed the hooks. On mismatch, auto-reinstalls for all previously
/// installed agents and editors.
pub fn session_start(agent: &dyn Agent) -> anyhow::Result<()> {
    let input = agent.parse_hook_input(HookType::SessionStart);

    // Delete session cache file
    if let Some(ref sid) = input.session_id
        && let Some(cp) = session_cache_path(sid)
    {
        let _ = fs::remove_file(cp); // Best-effort: stale cache file may not exist
    }

    // Clean up orphan "session moved" marker files.
    clean_moved_markers();

    // Auto-upgrade hooks on version mismatch. Runs at most once per binary
    // version: the version is saved after the attempt regardless of outcome.
    auto_upgrade_hooks();

    // Check whether this session is actively listening for narration.
    let listening = state::listening_session();
    let is_listening = input.session_id.is_some() && listening == input.session_id;

    agent.session_start(&input, is_listening)
}

/// Handle the `UserPromptSubmit` hook: emit editor context if changed.
///
/// When the prompt is `/attend`, activates narration mode instead of
/// emitting editor context.
pub fn user_prompt(agent: &dyn Agent, cli_cwd: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let input = agent.parse_hook_input(HookType::UserPrompt);

    // Check for /attend activation
    if is_attend_prompt(&input) {
        let session_id = input
            .session_id
            .ok_or_else(|| anyhow::anyhow!("no session_id in hook input"))?;

        let Some(path) = state::listening_path() else {
            return Err(anyhow::anyhow!("cannot determine cache directory"));
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::util::atomic_write(&path, |file| {
            use std::io::Write;
            file.write_all(session_id.as_str().as_bytes())
        })?;

        // Clear any stale "session moved" marker so this session gets
        // a fresh notification if narration moves away again later.
        clear_session_moved_marker(&session_id);

        return agent.attend_activate(&session_id);
    }

    let cwd = cli_cwd.or(input.cwd).unwrap_or_else(|| {
        Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
            .unwrap_or_else(|_| Utf8PathBuf::from("."))
    });

    let config = crate::config::Config::load(&cwd);

    // Per-session cache: what this session last saw, used for deduplication.
    let session_previous = input
        .session_id
        .as_ref()
        .and_then(session_cache_path)
        .and_then(|cp| fs::read_to_string(&cp).ok())
        .and_then(|s| serde_json::from_str::<EditorState>(&s).ok());

    let state = match EditorState::current(Some(&cwd), &config.include_dirs)? {
        Some(s) => s,
        None => return Ok(()),
    };

    // If this session already saw this exact state, suppress output.
    if session_previous.as_ref() == Some(&state) {
        return Ok(());
    }

    // Update session cache and emit.
    if let Some(ref sid) = input.session_id
        && let Some(cp) = session_cache_path(sid)
    {
        if let Some(parent) = cp.parent() {
            let _ = fs::create_dir_all(parent); // Best-effort: will fail at write if missing
        }
        if let Err(e) = crate::util::atomic_write(&cp, |file| {
            serde_json::to_writer(io::BufWriter::new(file), &state).map_err(io::Error::other)
        }) {
            tracing::warn!("Failed to write session cache: {e}");
        }
    }

    agent.editor_context(&state)
}

/// Handle narration delivery hooks: Stop, PreToolUse, PostToolUse.
///
/// All three share the same check-and-deliver logic with ToolUse-specific
/// guards: `attend listen` is approved when the session is active (to start
/// the receiver) and blocked when the session was stolen (to prevent
/// steal-back). A running receiver is detected early to save a round trip.
pub fn check_narration(agent: &dyn Agent, hook_type: HookType) -> anyhow::Result<()> {
    let input = agent.parse_hook_input(hook_type);
    let listening = state::listening_session();

    // Resolve whether we are the active listening session.
    let is_active = matches!(
        (&listening, &input.session_id),
        (Some(l), Some(h)) if l == h
    );

    // ToolUse hooks: gate `attend listen` to avoid unnecessary round trips.
    // - Stolen session → block immediately (prevents steal-back bounce)
    // - Active session + pending narration → deliver as approve (receiver
    //   starts in the same round trip)
    // - Active session + no pending + receiver running → block with guidance
    // - Active session + no pending + no receiver → approve (let it start)
    let is_stolen = matches!(
        (&listening, &input.session_id),
        (Some(_), Some(_)) if !is_active
    );
    if is_stolen && is_attend_listen(&input) {
        return agent.attend_result(
            &HookDecision::block(GuidanceReason::SessionMoved),
            hook_type,
        );
    }
    if is_active && is_attend_listen(&input) {
        // Check for pending narration: deliver via hook and approve the
        // listen command so the receiver starts in the same round trip.
        let session_id = listening.as_ref().unwrap();
        let cwd = resolve_cwd(&input);
        let config = crate::config::Config::load(&cwd);
        let files = crate::narrate::receive::collect_pending(session_id);
        if let Some(content) =
            crate::narrate::receive::read_pending(&files, &cwd, &config.include_dirs)
        {
            crate::narrate::receive::archive_pending(&files, session_id);
            crate::narrate::receive::auto_prune(&config);
            return agent.attend_result(
                &HookDecision::PendingNarration {
                    content,
                    effect: GuidanceEffect::Approve,
                },
                hook_type,
            );
        }
        if receiver_alive() {
            return agent.attend_result(
                &HookDecision::block(GuidanceReason::ListenerAlreadyActive),
                hook_type,
            );
        }
        return agent.attend_result(&HookDecision::Silent, hook_type);
    }
    let (pending_content, pending_files) = if is_active {
        let session_id = listening.as_ref().unwrap();
        let cwd = resolve_cwd(&input);
        let config = crate::config::Config::load(&cwd);
        let files = crate::narrate::receive::collect_pending(session_id);
        let content = crate::narrate::receive::read_pending(&files, &cwd, &config.include_dirs);
        (content, files)
    } else {
        (None, Vec::new())
    };

    let stop_hook_active =
        matches!(input.kind, HookKind::Stop { stop_hook_active } if stop_hook_active);

    let mut decision = hook_decision(
        input.session_id.as_ref(),
        listening.as_ref(),
        pending_content,
        receiver_alive(),
        stop_hook_active,
    );

    // Stop hook cannot deliver narration content directly (no
    // `additionalContext` — `reason` is shown to the user). Convert
    // PendingNarration → NarrationReady guidance so the agent starts a
    // receiver, which triggers PreToolUse delivery.  The pending files
    // are intentionally NOT archived here; PreToolUse will consume them.
    if matches!(hook_type, HookType::Stop)
        && matches!(decision, HookDecision::PendingNarration { .. })
    {
        decision = HookDecision::block(GuidanceReason::NarrationReady);
    }

    // hook_decision returns Block for all non-Silent guidance. Convert to
    // Approve where the hook type warrants it:
    // - SessionMoved on Stop → Approve (let Claude stop, just inject guidance)
    // - StartReceiver on PreToolUse/PostToolUse → Approve (let tool through, inject nudge)
    if matches!(hook_type, HookType::Stop)
        && matches!(
            decision,
            HookDecision::Guidance {
                reason: GuidanceReason::SessionMoved,
                ..
            }
        )
    {
        decision = HookDecision::approve(GuidanceReason::SessionMoved);
    }
    if matches!(hook_type, HookType::PreToolUse | HookType::PostToolUse)
        && matches!(
            decision,
            HookDecision::Guidance {
                reason: GuidanceReason::StartReceiver,
                ..
            }
        )
    {
        decision = HookDecision::approve(GuidanceReason::StartReceiver);
    }

    // SessionMoved ratchet: deliver the guidance once per session, then
    // suppress on all subsequent hooks (Stop and ToolUse alike). The
    // PreToolUse block on `attend listen` independently prevents the
    // agent from stealing the session back.
    if matches!(
        decision,
        HookDecision::Guidance {
            reason: GuidanceReason::SessionMoved,
            ..
        }
    ) && let Some(ref sid) = input.session_id
    {
        if session_moved_already_notified(sid) {
            decision = HookDecision::Silent;
        } else {
            mark_session_moved_notified(sid);
        }
    }

    // Archive pending files when delivering narration.
    if matches!(&decision, HookDecision::PendingNarration { .. })
        && !pending_files.is_empty()
        && let Some(ref sid) = listening
    {
        crate::narrate::receive::archive_pending(&pending_files, sid);
        let cwd = resolve_cwd(&input);
        let config = crate::config::Config::load(&cwd);
        crate::narrate::receive::auto_prune(&config);
    }

    agent.attend_result(&decision, hook_type)
}

/// Resolve the working directory from hook input, falling back to the
/// process working directory.
fn resolve_cwd(input: &HookInput) -> Utf8PathBuf {
    input.cwd.clone().unwrap_or_else(|| {
        Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
            .unwrap_or_else(|_| Utf8PathBuf::from("."))
    })
}

#[cfg(test)]
mod tests;
