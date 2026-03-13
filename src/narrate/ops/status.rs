//! Show narration system status.
//!
//! The status display is split into two phases:
//! - [`query_status`] gathers all system state into a [`StatusInfo`] value.
//! - [`Display`] for `StatusInfo` formats the output.
//!
//! This separation allows callers to inspect the status programmatically
//! without parsing printed text, and makes the formatting independently
//! testable.

use std::fmt;
use std::fs;

use camino::{Utf8Path, Utf8PathBuf};
use native_messaging::install::{manifest, paths::Scope};

use crate::config::Config;
use crate::narrate::record::DaemonStatus;
use crate::narrate::transcribe::Engine;
use crate::narrate::{
    lock_owner_alive, pending_dir, receive_lock_path, record_lock_path, status_path,
};

/// Column width for label alignment (accommodates "Accessibility:").
const COL: usize = 16;

/// Column width for label alignment in the Paths sub-section.
const PATH_COL: usize = 12;

// ── Supporting types ────────────────────────────────────────────────────────

/// Recording daemon state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RecordingState {
    /// Lock file absent: daemon is not running.
    Stopped,
    /// Daemon alive, status file says "idle".
    Idle,
    /// Daemon alive, status file says "recording" (or status file missing).
    Recording,
    /// Daemon alive, status file says "paused".
    Paused,
    /// Lock file present, parseable, but owner process is dead.
    StaleLock,
    /// Lock file present but content is unparseable or unreadable.
    Unknown,
}

/// Receive listener state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ListenerState {
    /// No lock file: listener is not running.
    Inactive,
    /// Lock file present and owner alive.
    Active,
    /// Lock file present, parseable, but owner process is dead.
    StaleLock,
    /// Lock file present but content is unparseable or unreadable.
    Unknown,
}

/// Accessibility (external selection capture) state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AccessibilityState {
    /// Platform backend available and permission granted.
    Ok,
    /// Platform backend available but permission not granted.
    PermissionNotGranted,
    /// No platform backend (e.g., Linux without AT-SPI).
    NotAvailable,
}

/// Health status for an editor, shell, or browser integration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct IntegrationHealth {
    /// Display name of the integration (e.g., "zed", "fish", "chrome").
    pub name: String,
    /// Empty if healthy; otherwise contains warning messages.
    pub warnings: Vec<String>,
}

/// Transcription engine info.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EngineInfo {
    /// Human-readable engine name (e.g., "Parakeet TDT").
    pub display_name: &'static str,
    /// Whether the model files are cached on disk.
    pub model_cached: bool,
}

/// Filesystem paths shown in the status output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusPaths {
    pub cache: Utf8PathBuf,
    pub archive: Utf8PathBuf,
    pub lock: Utf8PathBuf,
    /// Global config file path, if the config home is available.
    pub config: Option<Utf8PathBuf>,
}

/// Complete snapshot of narration system status.
///
/// Produced by [`query_status`], rendered by [`Display`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StatusInfo {
    pub recording: RecordingState,
    pub engine: EngineInfo,
    pub idle_timeout: String,
    pub session: Option<String>,
    pub listener: ListenerState,
    pub editors: Vec<IntegrationHealth>,
    pub shells: Vec<IntegrationHealth>,
    pub browsers: Vec<IntegrationHealth>,
    pub accessibility: AccessibilityState,
    pub clipboard_enabled: bool,
    pub pending_count: usize,
    pub archive_size: u64,
    pub paths: StatusPaths,
    pub config_warnings: Vec<String>,
}

// ── Query ───────────────────────────────────────────────────────────────────

/// Gather all system state into a [`StatusInfo`] snapshot.
pub(crate) fn query_status() -> anyhow::Result<StatusInfo> {
    let cwd = Utf8PathBuf::try_from(std::env::current_dir().unwrap_or_default())
        .unwrap_or_else(|_| Utf8PathBuf::from("."));
    let config = Config::load(&cwd);

    // Recording state
    let lock_path = record_lock_path();
    let recording = query_recording_state(&lock_path);

    // Engine / model status
    let engine_variant = config.engine.unwrap_or(Engine::Parakeet);
    let model_path = config
        .model
        .clone()
        .unwrap_or_else(|| engine_variant.default_model_path());
    let engine = EngineInfo {
        display_name: engine_variant.display_name(),
        model_cached: engine_variant.is_model_cached(&model_path),
    };

    // Idle timeout
    let idle_timeout = match config.daemon_idle_timeout.as_deref() {
        Some("forever") => "forever".to_string(),
        Some(s) => s.to_string(),
        None => "5m (default)".to_string(),
    };

    // Session
    let session = crate::state::listening_session();
    let session_str = session.as_ref().map(|s| s.as_str().to_string());

    // Receive listener
    let recv_lock = receive_lock_path();
    let listener = query_listener_state(&recv_lock);

    // Editor integration health
    let mut editors = Vec::new();
    for editor in crate::editor::EDITORS {
        let warnings = editor.check_narration()?;
        editors.push(IntegrationHealth {
            name: editor.name().to_string(),
            warnings,
        });
    }

    // Shell integration health
    let meta = crate::state::installed_meta();
    let mut shells = Vec::new();
    if let Some(ref meta) = meta {
        for name in &meta.shells {
            if let Some(sh) = crate::shell::shell_by_name(name) {
                let warnings = sh.check()?;
                shells.push(IntegrationHealth {
                    name: name.clone(),
                    warnings,
                });
            }
        }
    }

    // Browser integration health (only show browsers with manifests installed)
    let mut browsers = Vec::new();
    for browser in crate::browser::BROWSERS {
        let name = browser.name();
        let manifest_ok =
            manifest::verify_installed("attend", Some(&[name]), Scope::User).unwrap_or(false);
        if manifest_ok {
            browsers.push(IntegrationHealth {
                name: name.to_string(),
                warnings: Vec::new(),
            });
        }
    }

    // Accessibility
    let accessibility = if let Some(source) = crate::narrate::ext_capture::platform_source() {
        if source.is_available() {
            AccessibilityState::Ok
        } else {
            AccessibilityState::PermissionNotGranted
        }
    } else {
        AccessibilityState::NotAvailable
    };

    // Clipboard
    let clipboard_enabled = config.clipboard_capture.unwrap_or(true);

    // Pending narration count
    let count_json = |dir: Utf8PathBuf| -> usize {
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
    let pending_count = session_count + local_count;

    // Archive size
    let archive_root = crate::narrate::narration_root().join("archive");
    let archive_size = dir_size_bytes(archive_root.as_std_path());

    // Paths
    let paths = StatusPaths {
        cache: crate::narrate::cache_dir(),
        archive: archive_root,
        lock: lock_path,
        config: crate::util::xdg_config_home().map(|dir| dir.join("attend").join("config.toml")),
    };

    // Config validation
    let mut config_warnings = Vec::new();
    if let Some(ref s) = config.archive_retention
        && s != "forever"
        && humantime::parse_duration(s).is_err()
    {
        config_warnings.push(format!(
            "archive_retention: invalid value {s:?} (using default 7d)"
        ));
    }
    if let Some(ref s) = config.daemon_idle_timeout
        && s != "forever"
        && humantime::parse_duration(s).is_err()
    {
        config_warnings.push(format!(
            "daemon_idle_timeout: invalid value {s:?} (using default 5m)"
        ));
    }
    if let Some(ref model) = config.model
        && !engine_variant.is_model_cached(model)
    {
        config_warnings.push(format!("model: custom path does not exist: {model}"));
    }
    if !config.include_dirs.is_empty() {
        for dir in &config.include_dirs {
            if !dir.exists() {
                config_warnings.push(format!("include_dirs: directory does not exist: {dir}"));
            }
        }
    }

    Ok(StatusInfo {
        recording,
        engine,
        idle_timeout,
        session: session_str,
        listener,
        editors,
        shells,
        browsers,
        accessibility,
        clipboard_enabled,
        pending_count,
        archive_size,
        paths,
        config_warnings,
    })
}

/// Determine recording state from the lock file and daemon status file.
fn query_recording_state(lock_path: &Utf8Path) -> RecordingState {
    if !lock_path.exists() {
        return RecordingState::Stopped;
    }
    match fs::read_to_string(lock_path) {
        Ok(content) => {
            if lock_owner_alive(&content) {
                // Read the daemon's status file for current state.
                match fs::read_to_string(status_path())
                    .ok()
                    .and_then(|s| s.trim().parse::<DaemonStatus>().ok())
                {
                    Some(DaemonStatus::Recording) => RecordingState::Recording,
                    Some(DaemonStatus::Paused) => RecordingState::Paused,
                    Some(DaemonStatus::Idle) => RecordingState::Idle,
                    None => RecordingState::Recording, // no status file: assume recording
                }
            } else if crate::narrate::parse_lock_content(content.trim()).is_some() {
                RecordingState::StaleLock
            } else {
                RecordingState::Unknown
            }
        }
        Err(_) => RecordingState::Unknown,
    }
}

/// Determine listener state from the receive lock file.
fn query_listener_state(recv_lock: &Utf8Path) -> ListenerState {
    if !recv_lock.exists() {
        return ListenerState::Inactive;
    }
    match fs::read_to_string(recv_lock) {
        Ok(content) => {
            if lock_owner_alive(&content) {
                ListenerState::Active
            } else if crate::narrate::parse_lock_content(content.trim()).is_some() {
                ListenerState::StaleLock
            } else {
                ListenerState::Unknown
            }
        }
        Err(_) => ListenerState::Unknown,
    }
}

// ── Display ─────────────────────────────────────────────────────────────────

impl fmt::Display for StatusInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Recording
        let recording = match &self.recording {
            RecordingState::Stopped => "stopped",
            RecordingState::Idle => "idle (daemon resident)",
            RecordingState::Recording => "recording",
            RecordingState::Paused => "paused",
            RecordingState::StaleLock => {
                "stale lock (daemon not running): run `attend narrate toggle` to clean up"
            }
            RecordingState::Unknown => "unknown (lock file unreadable)",
        };
        writeln!(f, "{:<COL$}{recording}", "Recording:")?;

        // Engine
        let model_status = if self.engine.model_cached {
            "downloaded"
        } else {
            "not downloaded"
        };
        writeln!(
            f,
            "{:<COL$}{} (model {model_status})",
            "Engine:", self.engine.display_name
        )?;

        // Idle timeout
        writeln!(f, "{:<COL$}{}", "Idle timeout:", self.idle_timeout)?;

        // Session
        writeln!(
            f,
            "{:<COL$}{}",
            "Session:",
            self.session.as_deref().unwrap_or("none")
        )?;

        // Listener
        let listener = match &self.listener {
            ListenerState::Inactive => "inactive",
            ListenerState::Active => "active",
            ListenerState::StaleLock => "stale lock",
            ListenerState::Unknown => "unknown (lock file unreadable)",
        };
        writeln!(f, "{:<COL$}{listener}", "Listener:")?;

        // Editors
        let editor_parts: Vec<String> = self
            .editors
            .iter()
            .map(|e| {
                if e.warnings.is_empty() {
                    format!("{} (ok)", e.name)
                } else {
                    format!("{} ({})", e.name, e.warnings.join("; "))
                }
            })
            .collect();
        if !editor_parts.is_empty() {
            writeln!(f, "{:<COL$}{}", "Editors:", editor_parts.join(", "))?;
        }

        // Shells
        let shell_parts: Vec<String> = self
            .shells
            .iter()
            .map(|s| {
                if s.warnings.is_empty() {
                    format!("{} (ok)", s.name)
                } else {
                    format!("{} ({})", s.name, s.warnings.join("; "))
                }
            })
            .collect();
        if !shell_parts.is_empty() {
            writeln!(f, "{:<COL$}{}", "Shells:", shell_parts.join(", "))?;
        }

        // Browsers
        if !self.browsers.is_empty() {
            let browser_parts: Vec<String> = self
                .browsers
                .iter()
                .map(|b| format!("{} (ok)", b.name))
                .collect();
            writeln!(f, "{:<COL$}{}", "Browsers:", browser_parts.join(", "))?;
        }

        // Accessibility
        let accessibility = match &self.accessibility {
            AccessibilityState::Ok => "ok",
            AccessibilityState::PermissionNotGranted => {
                "permission not granted (System Settings > Privacy & Security > Accessibility)"
            }
            AccessibilityState::NotAvailable => "not available (no platform backend)",
        };
        writeln!(f, "{:<COL$}{accessibility}", "Accessibility:")?;

        // Clipboard
        let clipboard = if self.clipboard_enabled {
            "enabled"
        } else {
            "disabled"
        };
        writeln!(f, "{:<COL$}{clipboard}", "Clipboard:")?;

        // Pending
        writeln!(f, "{:<COL$}{} narration(s)", "Pending:", self.pending_count)?;

        // Archive
        writeln!(f, "{:<COL$}{}", "Archive:", format_size(self.archive_size))?;

        // Paths
        writeln!(f)?;
        writeln!(f, "Paths:")?;
        writeln!(f, "  {:<PATH_COL$}{}", "Cache:", self.paths.cache)?;
        writeln!(f, "  {:<PATH_COL$}{}", "Archive:", self.paths.archive)?;
        writeln!(f, "  {:<PATH_COL$}{}", "Lock:", self.paths.lock)?;
        if let Some(ref config_path) = self.paths.config {
            writeln!(f, "  {:<PATH_COL$}{}", "Config:", config_path)?;
        }

        // Config warnings
        if !self.config_warnings.is_empty() {
            writeln!(f)?;
            writeln!(f, "Config warnings:")?;
            for w in &self.config_warnings {
                writeln!(f, "  - {w}")?;
            }
        }

        Ok(())
    }
}

// ── Public entry point ──────────────────────────────────────────────────────

/// Show recording and system status.
pub(crate) fn status() -> anyhow::Result<()> {
    let info = query_status()?;
    print!("{info}");
    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

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

#[cfg(test)]
mod tests;
