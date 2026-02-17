use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use crate::state;

/// Return the platform cache directory for session deduplication files.
fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("attend"))
}

/// Return the cache file path for a given session ID.
fn cache_path(session_id: &str) -> Option<PathBuf> {
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

    // Delete cache file
    if let Some(sid) = session_id
        && let Some(cp) = cache_path(sid)
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
        .and_then(|v| v.as_str())
        .map(String::from);
    let stdin_cwd = stdin_json
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    let cwd = cli_cwd
        .or(stdin_cwd)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let previous = session_id
        .as_deref()
        .and_then(cache_path)
        .and_then(|cp| fs::read_to_string(&cp).ok())
        .and_then(|s| serde_json::from_str::<state::EditorState>(&s).ok());

    let state = match state::EditorState::current(Some(&cwd), previous.as_ref())? {
        Some(s) => s,
        None => return Ok(()),
    };

    // If unchanged from cache, suppress output
    if previous.as_ref() == Some(&state) {
        return Ok(());
    }

    // Write cache and emit
    if let Some(sid) = &session_id
        && let Some(cp) = cache_path(sid)
    {
        if let Some(parent) = cp.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&cp, serde_json::to_string(&state).unwrap_or_default());
    }

    println!("<editor-context>\n{state}\n</editor-context>");
    Ok(())
}
