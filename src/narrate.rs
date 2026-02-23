//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.

mod audio;
mod capture;
mod clean;
mod diff_capture;
mod editor_capture;
mod ext_capture;
pub(crate) mod merge;
pub(crate) mod receive;
pub(crate) mod record;
pub(crate) mod render;
mod silence;
mod status;
pub(crate) mod transcribe;

use camino::Utf8PathBuf;

use crate::state::SessionId;

pub(crate) use clean::clean;
#[cfg(test)]
pub(crate) use clean::clean_archive_dir;
pub(crate) use status::status;

/// Check whether a process with the given PID is alive.
///
/// Sends signal 0 (no-op) via `kill(2)`: returns true if the process exists
/// and we have permission to signal it.
pub(crate) fn process_alive(pid: i32) -> bool {
    use nix::sys::signal;
    use nix::unistd::Pid;
    signal::kill(Pid::from_raw(pid), None).is_ok()
}

/// Base directory for all narration state files.
pub(crate) fn cache_dir() -> Utf8PathBuf {
    crate::state::cache_dir().expect("cannot determine cache directory")
}

/// Path to the record lock file.
pub(crate) fn record_lock_path() -> Utf8PathBuf {
    cache_dir().join("record.lock")
}

/// Path to the stop sentinel file.
pub(crate) fn stop_sentinel_path() -> Utf8PathBuf {
    cache_dir().join("stop")
}

/// Path to the flush sentinel file.
pub(crate) fn flush_sentinel_path() -> Utf8PathBuf {
    cache_dir().join("flush")
}

/// Path to the receive lock file.
pub(crate) fn receive_lock_path() -> Utf8PathBuf {
    cache_dir().join("receive.lock")
}

/// Directory where pending narration files are written.
///
/// Each narration is stored as `<timestamp>.json` inside
/// `<cache_dir>/attend/pending/<session_id>/` (platform cache directory).
pub(crate) fn pending_dir(session_id: &SessionId) -> Utf8PathBuf {
    cache_dir().join("pending").join(session_id.as_str())
}

/// Directory where archived narration files are stored.
pub(crate) fn archive_dir(session_id: &SessionId) -> Utf8PathBuf {
    cache_dir().join("archive").join(session_id.as_str())
}

/// Directory where the browser bridge stages selection events.
///
/// Events in this directory are not delivered directly to the agent.
/// Instead, the recording daemon collects them during flush/stop and
/// includes them in the narration output.
pub(crate) fn browser_staging_dir(session_id: &SessionId) -> Utf8PathBuf {
    cache_dir()
        .join("browser-staging")
        .join(session_id.as_str())
}

/// Collect and remove all staged browser selection events for a session.
///
/// Returns the events sorted by filename (timestamp). Files are removed
/// after reading (best-effort).
pub(crate) fn collect_browser_staging(session_id: &SessionId) -> Vec<merge::Event> {
    let dir = browser_staging_dir(session_id);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();

    let mut events = Vec::new();
    for path in &files {
        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(file_events) = serde_json::from_str::<Vec<merge::Event>>(&content)
        {
            events.extend(file_events);
        }
        // Remove after reading (best-effort).
        let _ = std::fs::remove_file(path);
    }

    events
}

/// Resolve the session ID from flag, listening file, or None.
pub(crate) fn resolve_session(flag: Option<String>) -> Option<SessionId> {
    flag.map(SessionId::from)
        .or_else(crate::state::listening_session)
}

#[cfg(test)]
mod tests;
