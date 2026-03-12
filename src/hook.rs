// Hook system: editor context injection and narration delivery.
//
// The hook binary is invoked by Claude Code at lifecycle boundaries
// (SessionStart, SessionEnd, UserPromptSubmit, Stop, PreToolUse,
// PostToolUse). Each invocation is stateless: all state lives in the
// filesystem (listening file, session markers, pending narration files).
//
// ## Sessions and ownership
//
// A "session" is a single Claude Code conversation, identified by a
// session ID provided in the hook input. One session at a time can own
// narration: its ID is written to the listening file. The owning session
// receives pending narration via its PreToolUse hooks.
//
// A session becomes the owner via:
// - Explicit `/attend` (UserPromptSubmit writes the listening file)
// - Auto-claim (`attend listen` PreToolUse on an Inactive session that
//   hasn't been displaced — lets agents self-activate)
//
// ## Displacement
//
// When session B activates while session A owns narration, A is
// "displaced": on its next hook, it receives a one-shot SessionMoved
// advisory, then goes silent. The displaced state is recorded as a
// marker file and acts as a ratchet: once displaced, a session cannot
// auto-reclaim. The user must type `/attend` again to re-activate.
//
// This ratchet prevents livelock: without it, two sessions could
// steal narration back and forth on alternating hooks.
//
// ## Narration delivery pipeline
//
// The recording daemon writes pending narration files. Delivery happens
// synchronously during PreToolUse of `attend listen`:
//
//   daemon (record.rs) -> writes pending files
//   -> PreToolUse(attend listen) reads and delivers content
//   -> archives pending files
//   -> approves so the background listener restarts
//
// For non-listen hooks, pending narration triggers a NarrationReady
// block that tells the agent to run `attend listen` to pick it up.
//
// ## Dispatch axes
//
// The dispatcher classifies each hook along three axes:
//
//   1. Session relation: Active | Stolen | Inactive
//   2. Listen command:   Listen | ListenStop | None
//   3. Hook type:        the six HookType variants
//
// Listen/ListenStop hooks route to handle_listen_hook and
// handle_unlisten_hook respectively. Everything else routes to
// handle_general_hook, which uses the pure general_decision()
// function for its logic.
//
// ## Decision table (general hooks, listen_cmd = None)
//
//   Relation  | has_pending | receiver_alive | Decision
//   ----------|-------------|----------------|---------
//   Inactive  | -           | -              | Silent
//   Stolen    | -           | -              | SessionMoved (once, then Silent)
//   Active    | true        | -              | Block(NarrationReady)
//   Active    | false       | true           | Silent
//   Active    | false       | false          | StartReceiver (Block on Stop, Approve otherwise)
//
// ## Decision table (listen hooks, listen_cmd = Listen)
//
//   Relation  | Hook phase   | Decision
//   ----------|--------------|-----------------------------------
//   Stolen    | PreToolUse   | Block(SessionMoved)
//   Active    | PreToolUse   | deliver pending, or Silent if none
//   Inactive  | PreToolUse   | auto-claim or Block(Deactivated)
//   any       | PostToolUse  | Approve(ListenerStarted)
//
// ## Decision table (unlisten hooks, listen_cmd = ListenStop)
//
//   Relation           | Hook phase   | Decision
//   -------------------|--------------|----------------------------
//   Active             | PreToolUse   | Approve(Deactivated)
//   Stolen / Inactive  | PreToolUse   | Block(SessionMoved)
//   any                | PostToolUse  | Silent

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
pub(crate) use session_state::{clear_session_displaced, mark_session_displaced};

// Internal imports from submodules.
#[cfg(test)]
use command::parse_listen_command;
use command::{ListenCommand, detect_listen_command, is_attend_prompt, is_unattend_prompt};
use decision::{SessionRelation, general_decision, receiver_alive};
use session_state::{
    clean_session_markers, mark_session_activated, session_cache_path, session_displaced,
    session_was_activated,
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

    // Clean up orphan session marker files (displaced-*, activated-*).
    clean_session_markers();

    // Auto-upgrade hooks on version mismatch. Runs at most once per binary
    // version: the version is saved after the attempt regardless of outcome.
    auto_upgrade_hooks();

    // Check whether this session is actively listening for narration.
    let listening = state::listening_session();
    let is_listening = input.session_id.is_some() && listening == input.session_id;

    agent.session_start(&input, is_listening)
}

/// Handle the `SessionEnd` hook: clean up session state.
///
/// Removes the listening file (if this session owns it) and cleans up
/// any leftover browser staging files. Produces no output.
pub fn session_end(agent: &dyn Agent) -> anyhow::Result<()> {
    let input = agent.parse_hook_input(HookType::SessionEnd);

    // Only clean up if this session owns the listening file.
    if let Some(ref sid) = input.session_id {
        let listening = state::listening_session();
        if listening.as_ref() == Some(sid)
            && let Some(path) = state::listening_path()
        {
            let _ = std::fs::remove_file(path);
        }

        // Clean up staging directories for this session (best-effort).
        for staging_fn in [
            crate::narrate::browser_staging_dir,
            crate::narrate::shell_staging_dir,
        ] {
            let _ = std::fs::remove_dir_all(staging_fn(Some(sid)));
            // Also clean _local staging (events captured before a session existed).
            let _ = std::fs::remove_dir_all(staging_fn(None));
        }
    }

    Ok(())
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

        activate_session(&session_id)?;
        return agent.attend_activate(&session_id);
    }

    // Check for /unattend deactivation
    if is_unattend_prompt(&input) {
        let session_id = input
            .session_id
            .ok_or_else(|| anyhow::anyhow!("no session_id in hook input"))?;

        let listening = state::listening_session();
        if listening.as_ref() == Some(&session_id) {
            mark_session_displaced(&session_id);
            if let Some(path) = state::listening_path() {
                let _ = fs::remove_file(path);
            }
        }

        return agent.attend_deactivate(&session_id);
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

    let listen_cmd = detect_listen_command(&input);

    // Sessions that never activated `/attend` don't participate in
    // narration: skip all hook logic so we don't block unrelated sessions.
    // Exception: `attend listen` bypasses the gate so the agent can
    // self-activate without the user typing /attend first.
    if relation != SessionRelation::Active
        && let Some(ref sid) = input.session_id
        && !session_was_activated(sid)
        && !matches!(listen_cmd, Some(ListenCommand::Listen))
    {
        return agent.attend_result(&HookDecision::Silent, hook_type);
    }

    match listen_cmd {
        Some(ListenCommand::Listen) => {
            handle_listen_hook(agent, hook_type, &input, relation, listening.as_ref())
        }
        Some(ListenCommand::ListenStop) => handle_unlisten_hook(agent, hook_type, &input, relation),
        None => handle_general_hook(agent, hook_type, &input, relation, listening.as_ref()),
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
            if let Some(content) = crate::narrate::receive::read_pending(
                &files,
                Some(&cwd),
                &config.include_dirs,
                crate::narrate::render::RenderMode::Agent,
            ) {
                crate::narrate::receive::archive_pending(&files, session_id);
                crate::narrate::receive::auto_prune(&config);
                return agent.deliver_narration(&content);
            }
            // Files exist but produced no deliverable content (filtered
            // out by cwd, empty, or malformed). Archive them so they
            // can't trigger a livelock if general_hook's check races
            // with this one (defense-in-depth: general_hook does the
            // same cleanup, but belt-and-suspenders here too).
            if !files.is_empty() {
                crate::narrate::receive::archive_pending(&files, session_id);
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
        SessionRelation::Inactive => {
            // Auto-claim: if the agent runs `attend listen` without an
            // active session, activate narration for this session so it
            // doesn't need the user to type /attend first.
            //
            // Guard: if the session was explicitly deactivated (or
            // displaced by another session), don't auto-reclaim. The
            // user must type /attend to re-activate.
            if matches!(hook_type, HookType::PreToolUse)
                && let Some(ref sid) = input.session_id
            {
                if session_displaced(sid) {
                    return agent.attend_result(
                        &HookDecision::block(GuidanceReason::Deactivated),
                        hook_type,
                    );
                }
                if let Err(e) = activate_session(sid) {
                    tracing::warn!("auto-claim failed: {e}");
                }
                return agent.attend_activate(sid);
            }
            agent.attend_result(&HookDecision::Silent, hook_type)
        }
    }
}

/// Handle PreToolUse/PostToolUse for `attend listen --stop`.
///
/// - PostToolUse: approve silently (the command already ran).
/// - Active session: remove listening file, approve with deactivation guidance.
/// - Stolen/Inactive: block (only the owning session can deactivate).
fn handle_unlisten_hook(
    agent: &dyn Agent,
    hook_type: HookType,
    _input: &HookInput,
    relation: SessionRelation,
) -> anyhow::Result<()> {
    // PostToolUse: the command already ran. Approve silently.
    if matches!(hook_type, HookType::PostToolUse) {
        return agent.attend_result(&HookDecision::Silent, hook_type);
    }

    // PreToolUse: gate whether deactivation is allowed.
    match relation {
        SessionRelation::Active => {
            // This session owns narration — mark displaced so the
            // auto-claim path won't re-activate. The actual listening
            // file removal happens in `stop()` (the command we're
            // approving), which is the sole owner of that mutation.
            if let Some(session_id) = state::listening_session() {
                mark_session_displaced(&session_id);
            }
            agent.attend_result(
                &HookDecision::approve(GuidanceReason::Deactivated),
                hook_type,
            )
        }
        SessionRelation::Stolen | SessionRelation::Inactive => {
            // Not this session's narration — block to prevent
            // cross-session interference.
            agent.attend_result(
                &HookDecision::block(GuidanceReason::SessionMoved),
                hook_type,
            )
        }
    }
}

/// Handle Stop/PreToolUse/PostToolUse for tools other than `attend listen`.
///
/// Calls `general_decision` for the pure logic, then applies the
/// displaced ratchet (deliver SessionMoved advisory once, suppress thereafter).
fn handle_general_hook(
    agent: &dyn Agent,
    hook_type: HookType,
    input: &HookInput,
    relation: SessionRelation,
    listening: Option<&state::SessionId>,
) -> anyhow::Result<()> {
    let has_pending = matches!(relation, SessionRelation::Active) && {
        let session_id = listening.unwrap();
        let cwd = resolve_cwd(input);
        let config = crate::config::Config::load(&cwd);
        let files = crate::narrate::receive::collect_pending(session_id);
        if files.is_empty() {
            false
        } else if crate::narrate::receive::read_pending(
            &files,
            Some(&cwd),
            &config.include_dirs,
            crate::narrate::render::RenderMode::Agent,
        )
        .is_some()
        {
            true
        } else {
            // Files exist but produced no deliverable content (filtered
            // out by cwd, empty, or malformed). Archive them so they
            // don't keep triggering NarrationReady blocks on every
            // subsequent hook, which would livelock the agent.
            crate::narrate::receive::archive_pending(&files, session_id);
            false
        }
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

    // Displaced ratchet: deliver the SessionMoved advisory once per
    // session, then suppress. The PreToolUse block on `attend listen`
    // independently prevents the agent from stealing the session back.
    if matches!(
        decision,
        HookDecision::Guidance {
            reason: GuidanceReason::SessionMoved,
            ..
        }
    ) && let Some(ref sid) = input.session_id
    {
        if session_displaced(sid) {
            decision = HookDecision::Silent;
        } else {
            mark_session_displaced(sid);
        }
    }

    agent.attend_result(&decision, hook_type)
}

/// Write the session ID to the listening file and mark the session as
/// activated. Shared by `/attend` (UserPromptSubmit) and auto-claim
/// (`attend listen` PreToolUse on Inactive).
fn activate_session(session_id: &state::SessionId) -> anyhow::Result<()> {
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
    // hooks know it participates. Clear any stale "displaced" marker
    // so this session gets a fresh notification if narration moves
    // away again later.
    mark_session_activated(session_id);
    clear_session_displaced(session_id);
    Ok(())
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
