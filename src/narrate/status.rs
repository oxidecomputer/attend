//! Show narration system status.

use std::fs;

use camino::Utf8PathBuf;
use native_messaging::install::{manifest, paths::Scope};

use super::transcribe::Engine;
use super::{
    lock_owner_alive, pause_sentinel_path, pending_dir, receive_lock_path, record_lock_path,
};
use crate::config::Config;

/// Column width for label alignment (accommodates "Accessibility:").
const COL: usize = 16;

/// Column width for label alignment in the Paths sub-section.
const PATH_COL: usize = 12;

/// Show recording and system status.
pub(crate) fn status() -> anyhow::Result<()> {
    let cwd = Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| Utf8PathBuf::from("."));
    let config = Config::load(&cwd);

    // Recording state
    let lock_path = record_lock_path();
    let recording = if lock_path.exists() {
        if let Ok(content) = fs::read_to_string(&lock_path) {
            if lock_owner_alive(&content) {
                if pause_sentinel_path().exists() {
                    "idle (daemon resident)"
                } else {
                    "recording"
                }
            } else if super::parse_lock_content(content.trim()).is_some() {
                "stale lock (daemon not running): run `attend narrate toggle` to clean up"
            } else {
                "unknown (lock file unreadable)"
            }
        } else {
            "unknown (lock file unreadable)"
        }
    } else {
        "stopped"
    };
    println!("{:<COL$}{recording}", "Recording:");

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
        "{:<COL$}{} (model {model_status})",
        "Engine:",
        engine.display_name()
    );

    // Idle timeout
    let idle_timeout = match config.daemon_idle_timeout.as_deref() {
        Some("forever") => "forever".to_string(),
        Some(s) => s.to_string(),
        None => "5m (default)".to_string(),
    };
    println!("{:<COL$}{idle_timeout}", "Idle timeout:");

    // Session
    let session = crate::state::listening_session();
    println!(
        "{:<COL$}{}",
        "Session:",
        session.as_ref().map_or("none", |s| s.as_str())
    );

    // Receive listener
    let recv_lock = receive_lock_path();
    let listener = if recv_lock.exists() {
        if let Ok(content) = fs::read_to_string(&recv_lock) {
            if lock_owner_alive(&content) {
                "active"
            } else if super::parse_lock_content(content.trim()).is_some() {
                "stale lock"
            } else {
                "unknown (lock file unreadable)"
            }
        } else {
            "unknown (lock file unreadable)"
        }
    } else {
        "inactive"
    };
    println!("{:<COL$}{listener}", "Listener:");

    // Editor integration health
    let mut editor_parts: Vec<String> = Vec::new();
    for editor in crate::editor::EDITORS {
        let warnings = editor.check_narration()?;
        if warnings.is_empty() {
            editor_parts.push(format!("{} (ok)", editor.name()));
        } else {
            editor_parts.push(format!("{} ({})", editor.name(), warnings.join("; ")));
        }
    }
    if !editor_parts.is_empty() {
        println!("{:<COL$}{}", "Editors:", editor_parts.join(", "));
    }

    // Shell integration health
    let meta = crate::state::installed_meta();
    let mut shell_parts: Vec<String> = Vec::new();
    if let Some(ref meta) = meta {
        for name in &meta.shells {
            if let Some(sh) = crate::shell::shell_by_name(name) {
                let warnings = sh.check()?;
                if warnings.is_empty() {
                    shell_parts.push(format!("{name} (ok)"));
                } else {
                    shell_parts.push(format!("{name} ({})", warnings.join("; ")));
                }
            }
        }
    }
    if !shell_parts.is_empty() {
        println!("{:<COL$}{}", "Shells:", shell_parts.join(", "));
    }

    // Browser integration health (only show browsers with manifests installed)
    let mut browser_parts: Vec<String> = Vec::new();
    for browser in crate::browser::BROWSERS {
        let name = browser.name();
        let manifest_ok =
            manifest::verify_installed("attend", Some(&[name]), Scope::User).unwrap_or(false);
        if manifest_ok {
            browser_parts.push(format!("{name} (ok)"));
        }
    }
    if !browser_parts.is_empty() {
        println!("{:<COL$}{}", "Browsers:", browser_parts.join(", "));
    }

    // Accessibility (external selection capture)
    let accessibility = if let Some(source) = super::ext_capture::platform_source() {
        if source.is_available() {
            "ok"
        } else {
            "permission not granted (System Settings > Privacy & Security > Accessibility)"
        }
    } else {
        "not available (no platform backend)"
    };
    println!("{:<COL$}{accessibility}", "Accessibility:");

    // Clipboard capture
    let clipboard = if config.clipboard_capture.unwrap_or(true) {
        "enabled"
    } else {
        "disabled"
    };
    println!("{:<COL$}{clipboard}", "Clipboard:");

    // Pending narration count (session + _local)
    let count_json = |dir: camino::Utf8PathBuf| -> usize {
        fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                    .count()
            })
            .unwrap_or(0)
    };
    let session_count = session
        .as_ref()
        .map(|sid| count_json(pending_dir(Some(sid))))
        .unwrap_or(0);
    let local_count = count_json(pending_dir(None));
    let count = session_count + local_count;
    println!("{:<COL$}{count} narration(s)", "Pending:");

    // Archive size
    let archive_root = super::narration_root().join("archive");
    let archive_size = dir_size_bytes(archive_root.as_std_path());
    println!("{:<COL$}{}", "Archive:", format_size(archive_size));

    // Useful paths
    println!();
    println!("Paths:");
    println!("  {:<PATH_COL$}{}", "Cache:", super::cache_dir());
    println!("  {:<PATH_COL$}{archive_root}", "Archive:");
    println!("  {:<PATH_COL$}{lock_path}", "Lock:");
    if let Some(global_dir) = crate::util::xdg_config_home() {
        println!(
            "  {:<PATH_COL$}{}",
            "Config:",
            global_dir.join("attend").join("config.toml")
        );
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
    if let Some(ref s) = config.daemon_idle_timeout
        && s != "forever"
        && humantime::parse_duration(s).is_err()
    {
        warnings.push(format!(
            "daemon_idle_timeout: invalid value {s:?} (using default 5m)"
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

/// Recursively compute total size of a directory in bytes.
fn dir_size_bytes(path: &std::path::Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let ft = entry.file_type();
            if ft.as_ref().is_ok_and(|t| t.is_dir()) {
                total += dir_size_bytes(&entry.path());
            } else if ft.as_ref().is_ok_and(|t| t.is_file()) {
                total += entry.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
    }
    total
}

/// Format a byte count as a human-readable size string.
fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
