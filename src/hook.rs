use std::fs;
use std::io;

use camino::Utf8PathBuf;

use crate::agent::Agent;
use crate::state::{self, EditorState, SessionId};

/// Parsed input from an agent hook invocation.
///
/// Each agent fills this from its own input source (e.g., Claude reads
/// stdin JSON). The shared orchestrator functions consume it.
#[derive(Debug, Default)]
pub struct HookInput {
    pub session_id: Option<SessionId>,
    pub cwd: Option<Utf8PathBuf>,
    /// User prompt text (UserPrompt hook only).
    pub prompt: Option<String>,
    /// Whether the stop hook was re-invoked after a previous block.
    pub stop_hook_active: bool,
}

/// Structured hook decision with semantic variants.
///
/// Produced by the shared `hook_decision` logic, consumed by each agent's
/// `attend_result` method to render agent-specific output.
#[derive(Debug, PartialEq)]
pub enum HookDecision {
    /// No output needed.
    Silent,
    /// Narration moved to another session.
    SessionMoved,
    /// Pending narration to deliver.
    PendingNarration { content: String },
    /// No receiver running: agent should start one.
    StartReceiver,
}

/// Per-session cache: tracks what was last emitted to a given session for deduplication.
fn session_cache_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(state::cache_dir()?.join(format!("cache-{session_id}.json")))
}

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
    let input = agent.parse_hook_input();

    // Delete session cache file
    if let Some(ref sid) = input.session_id
        && let Some(cp) = session_cache_path(sid)
    {
        let _ = fs::remove_file(cp); // Best-effort: stale cache file may not exist
    }

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
    let input = agent.parse_hook_input();

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

/// Handle the `Stop` hook: deliver pending narration when the session stops.
pub fn stop(agent: &dyn Agent) -> anyhow::Result<()> {
    let input = agent.parse_hook_input();
    let listening = state::listening_session();

    // Resolve pending narration content (only if we're the active session).
    let is_active = matches!(
        (&listening, &input.session_id),
        (Some(l), Some(h)) if l == h
    );
    let (pending_content, pending_files) = if is_active {
        let session_id = listening.as_ref().unwrap();
        let cwd = input.cwd.clone().unwrap_or_else(|| {
            Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
                .unwrap_or_else(|_| Utf8PathBuf::from("."))
        });
        let config = crate::config::Config::load(&cwd);
        let files = crate::narrate::receive::collect_pending(session_id);
        let content = crate::narrate::receive::read_pending(&files, &cwd, &config.include_dirs);
        (content, files)
    } else {
        (None, Vec::new())
    };

    let decision = hook_decision(
        input.session_id.as_ref(),
        listening.as_ref(),
        pending_content,
        receiver_alive(),
        input.stop_hook_active,
    );

    // Archive pending files when delivering narration.
    if matches!(&decision, HookDecision::PendingNarration { .. })
        && !pending_files.is_empty()
        && let Some(ref sid) = listening
    {
        crate::narrate::receive::archive_pending(&pending_files, sid);
    }

    agent.attend_result(&decision)
}

/// Check if the user prompt is `/attend`.
fn is_attend_prompt(input: &HookInput) -> bool {
    input
        .prompt
        .as_deref()
        .is_some_and(|p| p.trim() == "/attend")
}

/// Pure decision logic for the stop hook.
///
/// Takes all external state as parameters so it can be tested without I/O.
///
/// `stop_hook_active` is set by Claude Code on re-invocation after a previous
/// block. We use it as a safety valve: if we already told the agent to start
/// a receiver and it's re-stopping, approve rather than risk an infinite
/// block loop (e.g. if the receiver hasn't created its lock file yet).
fn hook_decision(
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
        (Some(_), Some(_)) => return HookDecision::SessionMoved,
        // No listening session at all — approve silently.
        _ => return HookDecision::Silent,
    }

    // We are the active session. Pending narration always takes priority —
    // deliver it regardless of stop_hook_active.
    if let Some(content) = pending_content {
        return HookDecision::PendingNarration { content };
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
    HookDecision::StartReceiver
}

/// Check whether a background `receive --wait` process is alive.
fn receiver_alive() -> bool {
    let lock_path = crate::narrate::receive_lock_path();
    let Ok(content) = fs::read_to_string(&lock_path) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<i32>() else {
        return false;
    };
    crate::narrate::process_alive(pid)
}

/// Auto-upgrade hooks and editor integration when the running binary version
/// doesn't match the version that originally installed the hooks.
fn auto_upgrade_hooks() {
    let Some(meta) = state::installed_meta() else {
        return;
    };
    let running = env!("CARGO_PKG_VERSION");
    if meta.version == running {
        return;
    }

    tracing::info!(
        installed = meta.version,
        running,
        "Version mismatch: reinstalling hooks"
    );

    let bin_cmd = match crate::agent::resolve_bin_cmd(meta.dev) {
        Ok(cmd) => cmd,
        Err(e) => {
            tracing::warn!("Cannot resolve bin command for auto-upgrade: {e}");
            return;
        }
    };

    for name in &meta.agents {
        if let Err(e) = crate::agent::install(name, None, meta.dev) {
            tracing::warn!(agent = name, "Auto-upgrade failed for agent: {e}");
        }
    }
    for name in &meta.editors {
        if let Some(ed) = crate::editor::editor_by_name(name)
            && let Err(e) = ed.install_narration(&bin_cmd)
        {
            tracing::warn!(editor = name, "Auto-upgrade failed for editor: {e}");
        }
    }

    state::save_install_meta(&state::InstallMeta {
        version: running.to_string(),
        agents: meta.agents,
        editors: meta.editors,
        dev: meta.dev,
        project_paths: meta.project_paths,
    });

    eprintln!("attend: hooks upgraded from {} to {running}", meta.version);
}

#[cfg(test)]
mod tests;
