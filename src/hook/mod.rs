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
use decision::{SessionRelation, general_decision, receiver_alive};
use session_state::{
    clean_session_markers, mark_session_activated, mark_session_moved_notified, session_cache_path,
    session_moved_already_notified, session_was_activated,
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

    // Clean up orphan session marker files (moved-*, activated-*).
    clean_session_markers();

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

        // Mark this session as having activated attend, so narration
        // hooks know it participates. Clear any stale "session moved"
        // marker so this session gets a fresh notification if narration
        // moves away again later.
        mark_session_activated(&session_id);
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
/// Dispatches to [`handle_listen_hook`] for `attend listen` tool calls
/// and [`handle_general_hook`] for everything else.
pub fn check_narration(agent: &dyn Agent, hook_type: HookType) -> anyhow::Result<()> {
    let input = agent.parse_hook_input(hook_type);
    let listening = state::listening_session();

    let relation = match (&listening, &input.session_id) {
        (Some(l), Some(h)) if l == h => SessionRelation::Active,
        (Some(_), Some(_)) => SessionRelation::Stolen,
        _ => SessionRelation::Inactive,
    };

    // Sessions that never activated `/attend` don't participate in
    // narration: skip all hook logic so we don't block unrelated sessions.
    if relation != SessionRelation::Active
        && let Some(ref sid) = input.session_id
        && !session_was_activated(sid)
    {
        return agent.attend_result(&HookDecision::Silent, hook_type);
    }

    if is_attend_listen(&input) {
        handle_listen_hook(agent, hook_type, &input, relation, listening.as_ref())
    } else {
        handle_general_hook(agent, hook_type, &input, relation, listening.as_ref())
    }
}

/// Handle PreToolUse/PostToolUse for `attend listen` specifically.
///
/// - PostToolUse: approve with advisory (the command already ran).
/// - Stolen session: block to prevent steal-back livelock.
/// - Active session: deliver pending narration if any, or let the
///   listener start if no receiver is running yet.
fn handle_listen_hook(
    agent: &dyn Agent,
    hook_type: HookType,
    input: &HookInput,
    relation: SessionRelation,
    listening: Option<&state::SessionId>,
) -> anyhow::Result<()> {
    // PostToolUse: the command already ran. Emit advisory so the agent
    // knows to restart (not read) the listener when its task notification
    // arrives. Without this, the startup race (lock file not yet written)
    // causes a spurious StartReceiver advisory.
    if matches!(hook_type, HookType::PostToolUse) {
        return agent.attend_result(
            &HookDecision::approve(GuidanceReason::ListenerStarted),
            hook_type,
        );
    }

    // PreToolUse: gate whether the receiver is allowed to start.
    match relation {
        SessionRelation::Stolen => {
            // Block to prevent a steal-back bounce between sessions.
            agent.attend_result(
                &HookDecision::block(GuidanceReason::SessionMoved),
                hook_type,
            )
        }
        SessionRelation::Active => {
            // Sole narration delivery path: read pending files, deliver
            // content, and approve so the receiver starts in the same
            // round trip.
            let session_id = listening.unwrap();
            let cwd = resolve_cwd(input);
            let config = crate::config::Config::load(&cwd);
            let files = crate::narrate::receive::collect_pending(session_id);
            if let Some(content) =
                crate::narrate::receive::read_pending(&files, &cwd, &config.include_dirs)
            {
                crate::narrate::receive::archive_pending(&files, session_id);
                crate::narrate::receive::auto_prune(&config);
                return agent.deliver_narration(&content);
            }
            if receiver_alive() {
                return agent.attend_result(
                    &HookDecision::block(GuidanceReason::ListenerAlreadyActive),
                    hook_type,
                );
            }
            // No pending, no receiver — let it start silently.
            agent.attend_result(&HookDecision::Silent, hook_type)
        }
        SessionRelation::Inactive => agent.attend_result(&HookDecision::Silent, hook_type),
    }
}

/// Handle Stop/PreToolUse/PostToolUse for tools other than `attend listen`.
///
/// Calls `general_decision` for the pure logic, then applies the
/// SessionMoved ratchet (deliver once, suppress thereafter).
fn handle_general_hook(
    agent: &dyn Agent,
    hook_type: HookType,
    input: &HookInput,
    relation: SessionRelation,
    listening: Option<&state::SessionId>,
) -> anyhow::Result<()> {
    let has_pending = matches!(relation, SessionRelation::Active) && {
        let session_id = listening.unwrap();
        !crate::narrate::receive::collect_pending(session_id).is_empty()
    };

    let stop_hook_active =
        matches!(input.kind, HookKind::Stop { stop_hook_active } if stop_hook_active);

    let mut decision = general_decision(
        relation,
        has_pending,
        receiver_alive(),
        stop_hook_active,
        hook_type,
    );

    // SessionMoved ratchet: deliver the advisory once per session, then
    // suppress. The PreToolUse block on `attend listen` independently
    // prevents the agent from stealing the session back.
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
