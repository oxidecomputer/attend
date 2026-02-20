use std::fs;
use std::io::{self, Read};

use camino::Utf8PathBuf;

use crate::state::{self, SessionId};

/// Per-session cache: tracks what was last emitted to a given session for deduplication.
fn session_cache_path(session_id: &SessionId) -> Option<Utf8PathBuf> {
    Some(state::cache_dir()?.join(format!("cache-{session_id}.json")))
}

/// Read stdin and parse as JSON, returning `None` on any failure.
fn read_stdin_json() -> Option<serde_json::Value> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

/// Handle the `SessionStart` hook: clear cache and emit format instructions.
///
/// On compact/clear, if this session is actively listening for narration,
/// re-emit the narration skill instructions so the agent knows to restart
/// its background receiver.
///
/// Also checks whether the running binary version matches the version that
/// installed the hooks. On mismatch, auto-reinstalls for all previously
/// installed agents and editors.
pub fn session_start() -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();
    let session_id: Option<SessionId> = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(SessionId::from);

    // Delete session cache file
    if let Some(ref sid) = session_id
        && let Some(cp) = session_cache_path(sid)
    {
        let _ = fs::remove_file(cp); // Best-effort: stale cache file may not exist
    }

    // Auto-upgrade hooks on version mismatch.
    auto_upgrade_hooks();

    let bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "attend".to_string());

    // Emit instructions (templated with the binary path)
    print!(include_str!("instructions.txt"), bin_cmd = bin);

    // If this session is actively listening for narration, re-emit the
    // narration skill instructions so the agent restarts its background
    // receiver after context compaction or clear.
    if session_id.is_some() && state::listening_session() == session_id {
        print!("{}", narration_instructions(&bin));
    }

    Ok(())
}

/// Handle the `UserPromptSubmit` hook: emit editor context if changed.
///
/// When the prompt is `/attend`, activates narration mode instead of
/// emitting editor context.
pub fn run(cli_cwd: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();

    // Check for /attend activation
    if let Some(ref json) = stdin_json
        && is_attend_prompt(json)
    {
        return handle_attend_activate(json);
    }

    let session_id: Option<SessionId> = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(SessionId::from);
    let stdin_cwd = stdin_json
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(|v| v.as_str())
        .map(Utf8PathBuf::from);

    let cwd = cli_cwd.or(stdin_cwd).unwrap_or_else(|| {
        Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
            .unwrap_or_else(|_| Utf8PathBuf::from("."))
    });

    let config = crate::config::Config::load(&cwd);

    // Per-session cache: what this session last saw, used for deduplication.
    let session_previous = session_id
        .as_ref()
        .and_then(session_cache_path)
        .and_then(|cp| fs::read_to_string(&cp).ok())
        .and_then(|s| serde_json::from_str::<state::EditorState>(&s).ok());

    let state = match state::EditorState::current(Some(&cwd), &config.include_dirs)? {
        Some(s) => s,
        None => return Ok(()),
    };

    // If this session already saw this exact state, suppress output.
    if session_previous.as_ref() == Some(&state) {
        return Ok(());
    }

    // Update session cache and emit.
    if let Some(ref sid) = session_id
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

    println!("<editor-context>\n{state}\n</editor-context>");
    Ok(())
}

/// Decision from the stop hook logic.
#[derive(Debug, PartialEq)]
enum StopDecision {
    /// Approve silently — no output needed.
    Silent,
    /// Approve with informational reason (e.g. session moved).
    Approve { reason: String },
    /// Block the stop with a reason (narration content or guidance).
    Block { reason: String },
}

/// Pure decision logic for the stop hook.
///
/// Takes all external state as parameters so it can be tested without I/O.
///
/// `stop_hook_active` is set by Claude Code on re-invocation after a previous
/// block. We use it as a safety valve: if we already told the agent to start
/// a receiver and it's re-stopping, approve rather than risk an infinite
/// block loop (e.g. if the receiver hasn't created its lock file yet).
fn stop_decision(
    hook_session_id: Option<&SessionId>,
    listening_session: Option<&SessionId>,
    pending_content: Option<String>,
    receiver_alive: bool,
    stop_hook_active: bool,
) -> StopDecision {
    match (listening_session, hook_session_id) {
        // We are the active listening session — check for narration.
        (Some(listening_sid), Some(hook_sid)) if listening_sid == hook_sid => {}
        // Narration is active in a different session — inform Claude.
        (Some(_), Some(_)) => {
            return StopDecision::Approve {
                reason: "Narration has moved to a session with another agent. \
                    Do not restart the background receiver. \
                    If the user wants narration in this session, they will type /attend."
                    .to_string(),
            };
        }
        // No listening session at all — approve silently.
        _ => return StopDecision::Silent,
    }

    // We are the active session. Pending narration always takes priority —
    // deliver it regardless of stop_hook_active.
    if let Some(content) = pending_content {
        return StopDecision::Block { reason: content };
    }

    // No narration. If a receiver is running, it will handle future delivery.
    if receiver_alive {
        return StopDecision::Silent;
    }

    // No receiver. On re-invocation after a previous block, approve to avoid
    // an infinite loop (the agent already got the "start receiver" message).
    if stop_hook_active {
        return StopDecision::Silent;
    }

    // First attempt, no receiver — ask the agent to start one.
    StopDecision::Block {
        reason: "Run `attend listen` in the background to wait for the next narration.".to_string(),
    }
}

/// Handle the `Stop` hook: deliver pending narration when the session stops.
pub fn stop() -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();

    let hook_session_id: Option<SessionId> = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(SessionId::from);

    let listening = state::listening_session();

    // Resolve pending narration content (only if we're the active session).
    let is_active = matches!(
        (&listening, &hook_session_id),
        (Some(l), Some(h)) if l == h
    );
    let (pending_content, pending_files) = if is_active {
        let session_id = listening.as_ref().unwrap();
        let cwd_str = stdin_json
            .as_ref()
            .and_then(|v| v.get("cwd"))
            .and_then(|v| v.as_str());
        let cwd = cwd_str.map(Utf8PathBuf::from).unwrap_or_else(|| {
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

    let stop_hook_active = stdin_json
        .as_ref()
        .and_then(|v| v.get("stop_hook_active"))
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"));

    let decision = stop_decision(
        hook_session_id.as_ref(),
        listening.as_ref(),
        pending_content,
        receiver_alive(),
        stop_hook_active,
    );

    match decision {
        StopDecision::Silent => {}
        StopDecision::Approve { reason } => {
            let response = serde_json::json!({ "decision": "approve", "reason": reason });
            println!("{}", serde_json::to_string(&response)?);
        }
        StopDecision::Block { reason } => {
            // Archive pending files if we blocked with narration content.
            if !pending_files.is_empty()
                && let Some(ref sid) = listening
            {
                crate::narrate::receive::archive_pending(&pending_files, sid);
            }
            let response = serde_json::json!({ "decision": "block", "reason": reason });
            println!("{}", serde_json::to_string(&response)?);
        }
    }

    Ok(())
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

/// Build narration skill instructions for re-emission after context compaction.
///
/// Uses `claude_skill_body.md` — the same body as the installed SKILL.md,
/// so the instructions stay consistent with the skill template.
fn narration_instructions(bin_cmd: &str) -> String {
    let body = format!(
        include_str!("agent/claude_skill_body.md"),
        bin_cmd = bin_cmd
    );
    format!("\n<narration-instructions>\n{body}</narration-instructions>\n")
}

/// Check if the user prompt is `/attend`.
fn is_attend_prompt(json: &serde_json::Value) -> bool {
    json.get("prompt")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p.trim() == "/attend")
}

/// Activate narration mode for this session.
fn handle_attend_activate(json: &serde_json::Value) -> anyhow::Result<()> {
    let session_id: SessionId = json
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no session_id in hook stdin"))?
        .into();

    let Some(path) = crate::state::listening_path() else {
        return Err(anyhow::anyhow!("cannot determine cache directory"));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::util::atomic_write(&path, |file| {
        use std::io::Write;
        file.write_all(session_id.as_str().as_bytes())
    })?;

    let response = serde_json::json!({
        "additionalContext": "Narration mode activated for this session. \
            Listening for voice input.\n\n\
            Run `attend listen` in the background to wait for narration."
    });
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
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
    });

    eprintln!("attend: hooks upgraded from {} to {running}", meta.version);
}

#[cfg(test)]
mod tests;
