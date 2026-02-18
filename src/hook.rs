use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use crate::state;

/// Return the platform cache directory.
fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("attend"))
}

/// Shared cache: tracks latest editor state across all sessions for recency ordering.
fn shared_cache_path() -> Option<PathBuf> {
    Some(cache_dir()?.join("latest.json"))
}

/// Per-session cache: tracks what was last emitted to a given session for deduplication.
fn session_cache_path(session_id: &str) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("cache-{session_id}.json")))
}

/// Read stdin and parse as JSON, returning `None` on any failure.
fn read_stdin_json() -> Option<serde_json::Value> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

/// Handle the `SessionStart` hook: clear cache and emit format instructions.
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

    // Emit instructions
    print!("{}", include_str!("instructions.txt"));
    Ok(())
}

/// Handle the `UserPromptSubmit` hook: emit editor context if changed.
pub fn run(cli_cwd: Option<PathBuf>) -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();
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

    // Shared cache: latest state across all sessions, used for recency ordering.
    let shared_previous = shared_cache_path()
        .and_then(|cp| fs::read_to_string(&cp).ok())
        .and_then(|s| serde_json::from_str::<state::EditorState>(&s).ok());

    // Per-session cache: what this session last saw, used for deduplication.
    let session_previous = session_id
        .and_then(session_cache_path)
        .and_then(|cp| fs::read_to_string(&cp).ok())
        .and_then(|s| serde_json::from_str::<state::EditorState>(&s).ok());

    let state = match state::EditorState::current(Some(&cwd), shared_previous.as_ref())? {
        Some(s) => s,
        None => return Ok(()),
    };

    // Always update shared cache so other sessions benefit from fresh ordering.
    if let Some(cp) = shared_cache_path() {
        if let Some(parent) = cp.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(file) = fs::File::create(&cp) {
            let _ = serde_json::to_writer(io::BufWriter::new(file), &state);
        }
    }

    // If this session already saw this exact state, suppress output.
    if session_previous.as_ref() == Some(&state) {
        return Ok(());
    }

    // Update session cache and emit.
    if let Some(sid) = session_id
        && let Some(cp) = session_cache_path(sid)
        && let Ok(file) = fs::File::create(&cp)
    {
        let _ = serde_json::to_writer(io::BufWriter::new(file), &state);
    }

    println!("<editor-context>\n{state}\n</editor-context>");
    Ok(())
}
