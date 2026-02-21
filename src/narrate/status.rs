//! Show narration system status.

use std::fs;

use camino::Utf8PathBuf;

use super::transcribe::Engine;
use super::{pending_dir, process_alive, receive_lock_path, record_lock_path};
use crate::config::Config;

/// Show recording and system status.
pub(crate) fn status() -> anyhow::Result<()> {
    let cwd = Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| Utf8PathBuf::from("."));
    let config = Config::load(&cwd);

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

    // Engine / model status (from config, not hardcoded)
    let engine = config.engine.unwrap_or(Engine::Parakeet);
    let model_path = config
        .model
        .clone()
        .unwrap_or_else(|| engine.default_model_path());
    let model_status = if engine.is_model_cached(&model_path) {
        "downloaded"
    } else {
        "not downloaded"
    };
    println!(
        "Engine:     {} (model {model_status})",
        engine.display_name()
    );

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

    // Config validation
    let mut warnings = Vec::new();
    if let Some(ref s) = config.archive_retention
        && s != "forever"
        && humantime::parse_duration(s).is_err()
    {
        warnings.push(format!(
            "archive_retention: invalid value {s:?} (using default 7d)"
        ));
    }
    if let Some(ref model) = config.model
        && !engine.is_model_cached(model)
    {
        warnings.push(format!("model: custom path does not exist: {model}"));
    }
    if !config.include_dirs.is_empty() {
        for dir in &config.include_dirs {
            if !dir.exists() {
                warnings.push(format!("include_dirs: directory does not exist: {dir}"));
            }
        }
    }

    if !warnings.is_empty() {
        println!();
        println!("Config warnings:");
        for w in &warnings {
            println!("  - {w}");
        }
    }

    Ok(())
}
