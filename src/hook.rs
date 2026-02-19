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
        && let Ok(file) = fs::File::create(&cp)
    {
        if let Err(e) = serde_json::to_writer(io::BufWriter::new(file), &state) {
            tracing::warn!("Failed to write session cache: {e}");
        }
    }

    println!("<editor-context>\n{state}\n</editor-context>");
    Ok(())
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
    fs::write(&path, session_id)?;

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
