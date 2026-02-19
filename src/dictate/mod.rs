//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.

mod audio;
pub(crate) mod merge;
pub(crate) mod receive;
pub(crate) mod record;
pub(crate) mod transcribe;

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Check whether a process with the given PID is alive.
///
/// # Safety
/// Calls `libc::kill(pid, 0)` which checks process existence without
/// sending a signal. This is the POSIX-specified way to probe for a process.
pub(crate) fn process_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Base directory for all dictation state files.
fn cache_dir() -> PathBuf {
    crate::state::cache_dir().expect("cannot determine cache directory")
}

/// Path to the record lock file.
pub(crate) fn record_lock_path() -> PathBuf {
    cache_dir().join("record.lock")
}

/// Path to the stop sentinel file.
pub(crate) fn stop_sentinel_path() -> PathBuf {
    cache_dir().join("stop")
}

/// Path to the flush sentinel file.
pub(crate) fn flush_sentinel_path() -> PathBuf {
    cache_dir().join("flush")
}

/// Path to the receive lock file.
pub(crate) fn receive_lock_path() -> PathBuf {
    cache_dir().join("receive.lock")
}

/// Directory where pending dictation files are written.
///
/// Each dictation is stored as `<timestamp>.json` inside
/// `~/.cache/attend/pending/<session_id>/`.
pub(crate) fn pending_dir(session_id: &str) -> PathBuf {
    cache_dir().join("pending").join(session_id)
}

/// Directory where archived dictation files are stored.
pub(crate) fn archive_dir(session_id: &str) -> PathBuf {
    cache_dir().join("archive").join(session_id)
}

/// Resolve the session ID from flag, listening file, or None.
pub(crate) fn resolve_session(flag: Option<String>) -> Option<String> {
    flag.or_else(crate::state::listening_session)
}

/// Run model benchmarks for all engines and model variants.
pub(crate) fn bench() -> anyhow::Result<()> {
    use transcribe::Engine;

    let models_dir = cache_dir().join("models");
    let samples = vec![0.0f32; 16000 * 5];

    for engine in &[Engine::Whisper, Engine::Parakeet] {
        for name in engine.model_names() {
            let path = models_dir.join(name);
            tracing::info!("Ensuring model: {name}");
            engine.preload(&path)?; // ensure + load to verify download
            tracing::info!("--- {name} ---");
            let mut transcriber = engine.ensure_and_load(&path)?;
            transcriber.bench(&samples);
        }
    }

    Ok(())
}

/// Show recording and system status.
pub(crate) fn status() -> anyhow::Result<()> {
    use transcribe::Engine;

    // Recording state
    let lock_path = record_lock_path();
    let recording = if lock_path.exists() {
        if let Ok(content) = fs::read_to_string(&lock_path)
            && let Ok(pid) = content.trim().parse::<i32>()
        {
            if process_alive(pid) {
                "recording"
            } else {
                "stale lock (daemon not running) — run `attend dictate toggle` to clean up"
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
    println!("Session:    {}", session.as_deref().unwrap_or("none"));

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
        let warnings = editor.check_dictation()?;
        if warnings.is_empty() {
            println!("Editor:     {} (ok)", editor.name());
        } else {
            println!("Editor:     {} ({})", editor.name(), warnings.join("; "));
        }
    }

    // Pending dictation count
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
        println!("Pending:    {count} dictation(s)");
    } else {
        println!("Pending:    -");
    }

    Ok(())
}

/// Remove archived dictation files older than the given duration.
pub(crate) fn clean(older_than: Duration) -> anyhow::Result<()> {
    let archive_root = cache_dir().join("archive");
    let Ok(sessions) = fs::read_dir(&archive_root) else {
        println!("No archive directory found.");
        return Ok(());
    };

    let cutoff = SystemTime::now() - older_than;
    let mut removed = 0u64;

    for entry in sessions.filter_map(|e| e.ok()) {
        let session_dir = entry.path();
        if !session_dir.is_dir() {
            continue;
        }

        let Ok(files) = fs::read_dir(&session_dir) else {
            continue;
        };

        for file in files.filter_map(|e| e.ok()) {
            let path = file.path();
            let dominated_by_cutoff = path
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .is_some_and(|mtime| mtime < cutoff);

            if dominated_by_cutoff && fs::remove_file(&path).is_ok() {
                removed += 1;
            }
        }

        // Remove session directory if now empty.
        if fs::read_dir(&session_dir).is_ok_and(|mut d| d.next().is_none()) {
            let _ = fs::remove_dir(&session_dir);
        }
    }

    let age = humantime::format_duration(older_than);
    println!("Removed {removed} archived dictation(s) older than {age}.");
    Ok(())
}

#[cfg(test)]
mod tests;
