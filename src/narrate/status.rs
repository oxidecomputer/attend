//! Show narration system status.

use std::fs;

use super::transcribe::Engine;
use super::{pending_dir, process_alive, receive_lock_path, record_lock_path};

/// Show recording and system status.
pub(crate) fn status() -> anyhow::Result<()> {
    // Recording state
    let lock_path = record_lock_path();
    let recording = if lock_path.exists() {
        if let Ok(content) = fs::read_to_string(&lock_path)
            && let Ok(pid) = content.trim().parse::<i32>()
        {
            if process_alive(pid) {
                "recording"
            } else {
                "stale lock (daemon not running): run `attend narrate toggle` to clean up"
            }
        } else {
            "recording"
        }
    } else {
        "idle"
    };
    println!("Recording:  {recording}");

    // Engine / model status
    let engine = Engine::Parakeet;
    let model_path = engine.default_model_path();
    let model_status = if model_path.exists() {
        "downloaded"
    } else {
        "not downloaded"
    };
    println!("Engine:     parakeet (model {model_status})");

    // Session
    let session = crate::state::listening_session();
    println!(
        "Session:    {}",
        session.as_ref().map_or("none", |s| s.as_str())
    );

    // Receive listener
    let recv_lock = receive_lock_path();
    let listener = if recv_lock.exists() {
        if let Ok(content) = fs::read_to_string(&recv_lock) {
            if let Ok(pid) = content.trim().parse::<i32>() {
                if process_alive(pid) {
                    "active"
                } else {
                    "stale lock"
                }
            } else {
                "active"
            }
        } else {
            "active"
        }
    } else {
        "inactive"
    };
    println!("Listener:   {listener}");

    // Editor integration health
    for editor in crate::editor::EDITORS {
        let warnings = editor.check_narration()?;
        if warnings.is_empty() {
            println!("Editor:     {} (ok)", editor.name());
        } else {
            println!("Editor:     {} ({})", editor.name(), warnings.join("; "));
        }
    }

    // Pending narration count
    if let Some(ref sid) = session {
        let dir = pending_dir(sid);
        let count = fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                    .count()
            })
            .unwrap_or(0);
        println!("Pending:    {count} narration(s)");
    } else {
        println!("Pending:    -");
    }

    Ok(())
}
