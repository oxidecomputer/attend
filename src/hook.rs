use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use crate::state;

/// Per-session cache: tracks what was last emitted to a given session for deduplication.
fn session_cache_path(session_id: &str) -> Option<PathBuf> {
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
/// On compact/clear, if this session is actively listening for dictation,
/// re-emit the dictation skill instructions so the agent knows to restart
/// its background receiver.
pub fn session_start() -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();
    let session_id = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str());

    // Delete session cache file
    if let Some(sid) = session_id
        && let Some(cp) = session_cache_path(sid)
    {
        let _ = fs::remove_file(cp);
    }

    let bin = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "attend".to_string());

    // Emit instructions (templated with the binary path)
    print!(include_str!("instructions.txt"), bin_cmd = bin);

    // If this session is actively listening for dictation, re-emit the
    // dictation skill instructions so the agent restarts its background
    // receiver after context compaction or clear.
    if let Some(sid) = session_id
        && state::listening_session().as_deref() == Some(sid)
    {
        print!("{}", dictation_instructions(&bin));
    }

    Ok(())
}

/// Handle the `UserPromptSubmit` hook: emit editor context if changed.
///
/// When the prompt is `/attend`, activates dictation mode instead of
/// emitting editor context.
pub fn run(cli_cwd: Option<PathBuf>) -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();

    // Check for /attend activation
    if let Some(ref json) = stdin_json
        && is_attend_prompt(json)
    {
        return handle_attend_activate(json);
    }

    let session_id = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str());
    let stdin_cwd = stdin_json
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    let cwd = cli_cwd
        .or(stdin_cwd)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let config = crate::config::Config::load(&cwd);

    // Per-session cache: what this session last saw, used for deduplication.
    let session_previous = session_id
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
    if let Some(sid) = session_id
        && let Some(cp) = session_cache_path(sid)
    {
        if let Err(e) = state::atomic_write(&cp, |file| {
            serde_json::to_writer(io::BufWriter::new(file), &state)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))
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
    /// Block the stop with a reason (dictation content or guidance).
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
    hook_session_id: Option<&str>,
    listening_session: Option<&str>,
    pending_content: Option<String>,
    receiver_alive: bool,
    stop_hook_active: bool,
) -> StopDecision {
    match (listening_session, hook_session_id) {
        // We are the active listening session — check for dictation.
        (Some(listening_sid), Some(hook_sid)) if listening_sid == hook_sid => {}
        // Dictation is active in a different session — inform Claude.
        (Some(_), Some(_)) => {
            return StopDecision::Approve {
                reason: "Dictation has moved to a session with another agent. \
                    Do not restart the background receiver. \
                    If the user wants dictation in this session, they will type /attend."
                    .to_string(),
            };
        }
        // No listening session at all — approve silently.
        _ => return StopDecision::Silent,
    }

    // We are the active session. Pending dictation always takes priority —
    // deliver it regardless of stop_hook_active.
    if let Some(content) = pending_content {
        return StopDecision::Block { reason: content };
    }

    // No dictation. If a receiver is running, it will handle future delivery.
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
        reason: "Run `attend dictate receive --wait` in the background to wait for the next dictation.".to_string(),
    }
}

/// Handle the `Stop` hook: deliver pending dictation when the session stops.
pub fn stop() -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();

    let hook_session_id = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let listening = state::listening_session();

    // Resolve pending dictation content (only if we're the active session).
    let is_active = matches!(
        (&listening, &hook_session_id),
        (Some(l), Some(h)) if l == h
    );
    let (pending_content, pending_files) = if is_active {
        let session_id = listening.as_deref().unwrap();
        let cwd_str = stdin_json
            .as_ref()
            .and_then(|v| v.get("cwd"))
            .and_then(|v| v.as_str());
        let cwd = cwd_str
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
        let config = crate::config::Config::load(&cwd);
        let files = crate::dictate::receive::collect_pending(session_id);
        let content =
            crate::dictate::receive::read_pending(&files, &cwd, &config.include_dirs);
        (content, files)
    } else {
        (None, Vec::new())
    };

    let stop_hook_active = stdin_json
        .as_ref()
        .and_then(|v| v.get("stop_hook_active"))
        .is_some_and(|v| v.as_bool() == Some(true) || v.as_str() == Some("true"));

    let decision = stop_decision(
        hook_session_id.as_deref(),
        listening.as_deref(),
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
            // Archive pending files if we blocked with dictation content.
            if !pending_files.is_empty() {
                if let Some(sid) = listening.as_deref() {
                    crate::dictate::receive::archive_pending(&pending_files, sid);
                }
            }
            let response = serde_json::json!({ "decision": "block", "reason": reason });
            println!("{}", serde_json::to_string(&response)?);
        }
    }

    Ok(())
}

/// Check whether a background `receive --wait` process is alive.
fn receiver_alive() -> bool {
    let lock_path = crate::dictate::receive_lock_path();
    let Ok(content) = fs::read_to_string(&lock_path) else {
        return false;
    };
    let Ok(pid) = content.trim().parse::<i32>() else {
        return false;
    };
    crate::dictate::process_alive(pid)
}

/// Build dictation skill instructions for re-emission after context compaction.
///
/// Uses `claude_skill_body.md` — the same body as the installed SKILL.md,
/// so the instructions stay consistent with the skill template.
fn dictation_instructions(bin_cmd: &str) -> String {
    let body = format!(
        include_str!("agent/claude_skill_body.md"),
        bin_cmd = bin_cmd
    );
    format!("\n<dictation-instructions>\n{body}</dictation-instructions>\n")
}

/// Check if the user prompt is `/attend`.
fn is_attend_prompt(json: &serde_json::Value) -> bool {
    json.get("prompt")
        .and_then(|v| v.as_str())
        .is_some_and(|p| p.trim() == "/attend")
}

/// Activate dictation mode for this session.
fn handle_attend_activate(json: &serde_json::Value) -> anyhow::Result<()> {
    let session_id = json
        .get("session_id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no session_id in hook stdin"))?;

    let Some(path) = crate::state::listening_path() else {
        return Err(anyhow::anyhow!("cannot determine cache directory"));
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    state::atomic_write(&path, |file| {
        use std::io::Write;
        file.write_all(session_id.as_bytes())
    })?;

    let response = serde_json::json!({
        "additionalContext": "Dictation mode activated for this session. \
            Listening for voice input.\n\n\
            Run `attend dictate receive --wait` in the background to wait for dictation."
    });
    println!("{}", serde_json::to_string(&response)?);
    Ok(())
}

#[cfg(test)]
mod tests;
