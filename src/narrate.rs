//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.
//!
//! # Cache directory layout
//!
//! All runtime state lives under `~/Library/Caches/attend/` (macOS). The
//! layout groups files by subsystem so `ls` shows a small number of
//! well-named directories:
//!
//! ```text
//! cache_dir/
//! ├── daemon/                  Recording daemon IPC
//! │   ├── lock                 PID lock file (daemon is running)
//! │   ├── command              CLI→daemon: atomic command file
//! │   └── status               Daemon→CLI: current daemon state
//! ├── narration/               Narration file lifecycle
//! │   ├── pending/<key>/       Awaiting delivery to an agent session
//! │   ├── yanked/<key>/        Written by yank (isolated from hook delivery)
//! │   └── archive/<key>/       Delivered or yanked, kept for retention
//! ├── staging/                 Event staging for daemon collection
//! │   ├── browser/<key>/       Browser extension selection events
//! │   ├── shell/<key>/         Shell hook command events
//! │   └── clipboard/<key>/     Clipboard images (PNG)
//! ├── hooks/                   Hook system shared state
//! │   ├── listening            Currently attending session ID
//! │   ├── receive.lock         PID lock for the receive listener
//! │   └── latest.json          Shared editor state ordering cache
//! ├── sessions/                Per-session state
//! │   ├── cache/<sid>.json
//! │   ├── displaced/<sid>
//! │   └── activated/<sid>
//! ├── models/                  ML model files
//! └── version.json             Install metadata
//! ```
//!
//! `<key>` is either a session ID or `_local` (no active session).

pub(crate) mod audio;
pub(crate) mod capture;
mod chime;
mod clean;
pub(crate) mod clipboard_capture;
mod diff_capture;
pub(crate) mod editor_capture;
pub(crate) mod ext_capture;
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
/// The daemon lock file (daemon is running) remains the sole gate for
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

/// Check whether the process at `pid` is the same one that was created at
/// `created_at` (Unix epoch seconds).
///
/// Guards against PID reuse: after `kill(pid, 0)` confirms the process
/// exists, we compare its start time (via `sysinfo`) against the timestamp
/// stored in the lock file. If the start time differs by more than 2 seconds,
/// the PID was recycled and the lock is stale.
///
/// Falls back to plain `process_alive()` if sysinfo cannot retrieve the
/// process start time (e.g., on platforms where `/proc` is unavailable).
pub(crate) fn process_alive_since(pid: i32, created_at: i64) -> bool {
    if !process_alive(pid) {
        return false;
    }

    use sysinfo::{ProcessRefreshKind, System};

    let proc_refresh = ProcessRefreshKind::nothing();
    let mut sys = System::new();
    let sysinfo_pid = sysinfo::Pid::from_u32(pid as u32);
    sys.refresh_processes_specifics(
        sysinfo::ProcessesToUpdate::Some(&[sysinfo_pid]),
        true,
        proc_refresh,
    );

    let Some(proc_info) = sys.process(sysinfo_pid) else {
        // sysinfo couldn't find the process; fall back to kill(2) result.
        return true;
    };

    let start_time = proc_info.start_time() as i64; // Unix epoch seconds
    (start_time - created_at).abs() <= 2
}

/// Build the lock file content string: `"PID:TIMESTAMP\n"`.
///
/// `TIMESTAMP` is the current Unix epoch in seconds. Callers write this
/// to the lock file so that later readers can detect PID reuse.
pub(crate) fn lock_file_content() -> String {
    let pid = std::process::id();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before Unix epoch")
        .as_secs();
    format!("{pid}:{now}")
}

/// Parse a lock file's content into `(pid, optional_timestamp)`.
///
/// Supports two formats:
/// - New: `"PID:TIMESTAMP"` where TIMESTAMP is Unix epoch seconds.
/// - Legacy: `"PID"` (no timestamp).
///
/// Returns `None` if the content cannot be parsed at all.
pub(crate) fn parse_lock_content(content: &str) -> Option<(i32, Option<i64>)> {
    let trimmed = content.trim();
    if let Some((pid_str, ts_str)) = trimmed.split_once(':') {
        let pid = pid_str.parse::<i32>().ok()?;
        let ts = ts_str.parse::<i64>().ok()?;
        Some((pid, Some(ts)))
    } else {
        let pid = trimmed.parse::<i32>().ok()?;
        Some((pid, None))
    }
}

/// Check whether the process described by a lock file is still alive.
///
/// Uses `process_alive_since()` when a creation timestamp is available
/// (new format), falling back to plain `process_alive()` for legacy
/// lock files that contain only a PID.
pub(crate) fn lock_owner_alive(content: &str) -> bool {
    match parse_lock_content(content) {
        Some((pid, Some(ts))) => process_alive_since(pid, ts),
        Some((pid, None)) => process_alive(pid),
        None => false,
    }
}

/// Base directory for all narration state files.
pub(crate) fn cache_dir() -> Utf8PathBuf {
    crate::state::cache_dir().expect("cannot determine cache directory")
}

/// Root directory for recording daemon IPC (lock, command, status).
pub(crate) fn daemon_dir() -> Utf8PathBuf {
    cache_dir().join("daemon")
}

/// Root directory for the narration file lifecycle (pending, archive, yanked).
pub(crate) fn narration_root() -> Utf8PathBuf {
    cache_dir().join("narration")
}

/// Root directory for event staging (browser, shell, clipboard).
pub(crate) fn staging_root() -> Utf8PathBuf {
    cache_dir().join("staging")
}

/// Path to the record lock file.
pub(crate) fn record_lock_path() -> Utf8PathBuf {
    daemon_dir().join("lock")
}

/// Path to the command file (CLI -> daemon).
///
/// The CLI writes a command string ("stop", "flush", "pause", "resume",
/// "yank") atomically. The daemon reads, acts, and removes the file.
pub(crate) fn command_path() -> Utf8PathBuf {
    daemon_dir().join("command")
}

/// Path to the status file (daemon -> CLI).
///
/// The daemon writes its current state ("recording", "paused", "idle")
/// atomically after each state transition. The CLI reads this to decide
/// what command to send.
pub(crate) fn status_path() -> Utf8PathBuf {
    daemon_dir().join("status")
}

/// Path to the receive lock file.
pub(crate) fn receive_lock_path() -> Utf8PathBuf {
    crate::state::hooks_dir()
        .expect("cannot determine cache directory")
        .join("receive.lock")
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
/// `narration/pending/<key>/` where `<key>` is the session ID
/// or `_local` when no agent session is active.
pub(crate) fn pending_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    narration_root().join("pending").join(dir_key(session_id))
}

/// Directory where archived narration files are stored.
pub(crate) fn archive_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    narration_root().join("archive").join(dir_key(session_id))
}

/// Directory where the browser bridge stages selection events.
///
/// Events in this directory are not delivered directly to the agent.
/// Instead, the recording daemon collects them during flush/stop and
/// includes them in the narration output.
pub(crate) fn browser_staging_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    staging_root().join("browser").join(dir_key(session_id))
}

/// Directory where the shell hook stages command events.
///
/// Events in this directory are not delivered directly to the agent.
/// Instead, the recording daemon collects them during flush/stop and
/// includes them in the narration output.
pub(crate) fn shell_staging_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    staging_root().join("shell").join(dir_key(session_id))
}

/// Root directory for all clipboard staging (across sessions).
///
/// Used by cleanup (walk all session subdirs) and by permission patterns
/// (wildcard matching across sessions).
pub(crate) fn clipboard_staging_root() -> Utf8PathBuf {
    staging_root().join("clipboard")
}

/// Directory where clipboard images are staged as PNG files.
///
/// Session-scoped like browser and shell staging. When no session is
/// active, falls back to `_local/`.
pub(crate) fn clipboard_staging_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    clipboard_staging_root().join(dir_key(session_id))
}

/// Directory where yanked narration files are written.
///
/// Yank writes here instead of `pending/` so the hook delivery path
/// never sees the content (no race between yank CLI and hook delivery).
pub(crate) fn yanked_dir(session_id: Option<&SessionId>) -> Utf8PathBuf {
    narration_root().join("yanked").join(dir_key(session_id))
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
    now: chrono::DateTime<chrono::Utc>,
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
        // Parse wall-clock timestamp from filename. Filenames may be:
        //   "2026-02-23T22-42-28Z.json"                         (legacy, second precision)
        //   "2026-02-23T22-42-28.123456789Z.json"               (nanosecond precision)
        //   "2026-02-23T22-42-28.123456789Z-<uuid>.json"        (nanosecond + uniqueness suffix)
        // The timestamp always ends at the first 'Z'.
        let file_time = path.file_stem().and_then(|s| s.to_str()).and_then(|s| {
            let ts_part = s.find('Z').map(|i| &s[..=i]).unwrap_or(s);
            chrono::NaiveDateTime::parse_from_str(ts_part, "%Y-%m-%dT%H-%M-%S%.fZ")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(ts_part, "%Y-%m-%dT%H-%M-%SZ"))
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

        let timestamp = file_time.unwrap_or(now);

        if let Ok(content) = std::fs::read_to_string(path)
            && let Ok(file_events) = serde_json::from_str::<Vec<merge::Event>>(&content)
        {
            for mut event in file_events {
                // Assign the file's UTC timestamp to all event types.
                match &mut event {
                    merge::Event::BrowserSelection {
                        timestamp: ts,
                        last_seen: ls,
                        ..
                    } => {
                        *ts = timestamp;
                        *ls = timestamp;
                    }
                    merge::Event::ShellCommand { timestamp: ts, .. } => *ts = timestamp,
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
    now: chrono::DateTime<chrono::Utc>,
) -> StagingResult {
    collect_staging(
        browser_staging_dir(session_id).as_ref(),
        period_start_utc,
        now,
    )
}

/// Collect staged shell command events for a session.
pub(crate) fn collect_shell_staging(
    session_id: Option<&SessionId>,
    period_start_utc: chrono::DateTime<chrono::Utc>,
    now: chrono::DateTime<chrono::Utc>,
) -> StagingResult {
    collect_staging(
        shell_staging_dir(session_id).as_ref(),
        period_start_utc,
        now,
    )
}

/// Resolve the session ID from flag, listening file, or None.
pub(crate) fn resolve_session(flag: Option<String>) -> Option<SessionId> {
    flag.map(SessionId::from)
        .or_else(crate::state::listening_session)
}

#[cfg(test)]
mod tests;
