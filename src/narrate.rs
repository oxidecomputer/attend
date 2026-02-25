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

use camino::{Utf8Path, Utf8PathBuf};

use crate::state::SessionId;

/// Directory name used for staging/pending when no agent session is active.
///
/// The `record.lock` file (daemon is running) remains the sole gate for
/// whether events are captured at all. This fallback only affects *where*
/// events are staged when the daemon is running but no agent session exists.
const LOCAL_DIR_NAME: &str = "_local";

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

/// Path to the pause sentinel file.
///
/// Exists = paused, absent = not paused. The CLI toggles this file;
/// the daemon checks each loop iteration.
pub(crate) fn pause_sentinel_path() -> Utf8PathBuf {
    cache_dir().join("pause")
}

/// Path to the receive lock file.
pub(crate) fn receive_lock_path() -> Utf8PathBuf {
    cache_dir().join("receive.lock")
}

/// Resolve the directory key from an optional session ID.
///
/// Returns the session ID string when present, or [`LOCAL_DIR_NAME`] when
/// no agent session is active.
fn dir_key(session_id: Option<&SessionId>) -> &str {
    session_id.map(SessionId::as_str).unwrap_or(LOCAL_DIR_NAME)
}

/// Directory where pending narration files are written.
///
/// Each narration is stored as `<timestamp>.json` inside
/// `<cache_dir>/attend/pending/<key>/` where `<key>` is the session ID
/// or `_local` when no agent session is active.
pub(crate) fn pending_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    cache_dir().join("pending").join(dir_key(session_id))
}

/// Directory where archived narration files are stored.
pub(crate) fn archive_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    cache_dir().join("archive").join(dir_key(session_id))
}

/// Directory where the browser bridge stages selection events.
///
/// Events in this directory are not delivered directly to the agent.
/// Instead, the recording daemon collects them during flush/stop and
/// includes them in the narration output.
pub(crate) fn browser_staging_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    cache_dir()
        .join("browser-staging")
        .join(dir_key(session_id))
}

/// Directory where the shell hook stages command events.
///
/// Events in this directory are not delivered directly to the agent.
/// Instead, the recording daemon collects them during flush/stop and
/// includes them in the narration output.
pub(crate) fn shell_staging_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    cache_dir().join("shell-staging").join(dir_key(session_id))
}

// ── Generalized staging infrastructure ──────────────────────────────────────

/// Collected staging events, with file paths for deferred cleanup.
#[derive(Default)]
pub(crate) struct StagingResult {
    pub events: Vec<merge::Event>,
    files: Vec<std::path::PathBuf>,
}

/// Deferred cleanup handle: holds file paths for removal after write.
pub(crate) struct StagingCleanup {
    files: Vec<std::path::PathBuf>,
}

impl StagingCleanup {
    /// Remove the staging files (call after narration is safely on disk).
    pub fn cleanup(self) {
        for path in &self.files {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl StagingResult {
    /// Split into events (for merging) and a cleanup handle (for deferred removal).
    pub fn take(self) -> (Vec<merge::Event>, StagingCleanup) {
        (self.events, StagingCleanup { files: self.files })
    }
}

/// Collect staged events from a directory.
///
/// File timestamps (from the filename) are assigned as UTC timestamps
/// on the events, so they sort correctly with all other event types.
///
/// Files are **not** removed until [`StagingCleanup::cleanup`] is called,
/// so a crash between collection and narration write does not lose events.
fn collect_staging(
    dir: &Utf8Path,
    period_start_utc: chrono::DateTime<chrono::Utc>,
) -> StagingResult {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return StagingResult::default();
    };

    let mut files: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();

    let mut events = Vec::new();
    for path in &files {
        // Parse wall-clock timestamp from filename (e.g., "2026-02-23T22-42-28Z.json").
        let file_time = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
            chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H-%M-%SZ")
                .ok()
                .map(|naive| naive.and_utc())
        });

        // Skip events that predate the current recording period.
        if let Some(ft) = file_time
            && ft < period_start_utc
        {
            let _ = std::fs::remove_file(path);
            continue;
        }

        let timestamp = file_time.unwrap_or(chrono::Utc::now());

        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(file_events) = serde_json::from_str::<Vec<merge::Event>>(&content)
        {
            for mut event in file_events {
                // Assign the file's UTC timestamp to all event types.
                match &mut event {
                    merge::Event::BrowserSelection { timestamp: ts, .. }
                    | merge::Event::ShellCommand { timestamp: ts, .. } => *ts = timestamp,
                    _ => {}
                }
                events.push(event);
            }
        }
    }

    StagingResult { events, files }
}

/// Collect staged browser selection events for a session.
pub(crate) fn collect_browser_staging(
    session_id: Option<&SessionId>,
    period_start_utc: chrono::DateTime<chrono::Utc>,
) -> StagingResult {
    collect_staging(browser_staging_dir(session_id).as_ref(), period_start_utc)
}

/// Collect staged shell command events for a session.
pub(crate) fn collect_shell_staging(
    session_id: Option<&SessionId>,
    period_start_utc: chrono::DateTime<chrono::Utc>,
) -> StagingResult {
    collect_staging(shell_staging_dir(session_id).as_ref(), period_start_utc)
}

/// Resolve the session ID from flag, listening file, or None.
pub(crate) fn resolve_session(flag: Option<String>) -> Option<SessionId> {
    flag.map(SessionId::from)
        .or_else(crate::state::listening_session)
}

#[cfg(test)]
mod tests;
