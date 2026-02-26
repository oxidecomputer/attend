//! Check for and deliver pending narration files to Claude Code.
//!
//! Narration files are stored as individual timestamped JSON files in
//! `<cache_dir>/attend/pending/<key>/` where `<key>` is the session ID
//! or `_local` when no agent session was active during recording. Each
//! file contains a `Vec<Event>` with absolute paths. On receive, events
//! are filtered to the current project directory (and any configured
//! `include_dirs`), paths are relativized, and the result is rendered as
//! markdown wrapped in `<narration>` tags.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

use super::merge::Event;
use super::render::{self, SnipConfig};
use super::transcribe::Engine;
use super::{archive_dir, pending_dir, receive_lock_path, resolve_session};
use crate::config::Config;
use crate::state::SessionId;

/// How often to poll for pending narration when waiting (ms).
const NARRATION_POLL_MS: u64 = 500;

/// Collect all pending narration files for a session, sorted by filename (timestamp).
///
/// Also collects files from the `_local` directory (narrations captured when
/// no agent session was active), so they are delivered when a session starts.
pub(crate) fn collect_pending(session_id: &SessionId) -> Vec<PathBuf> {
    let mut files = collect_pending_dir(&pending_dir(Some(session_id)));
    // Also collect from _local (no-session narrations).
    files.extend(collect_pending_dir(&pending_dir(None)));
    files.sort();
    files
}

/// Collect `.json` files from a single pending directory.
fn collect_pending_dir(dir: &Utf8Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

/// Deserialize, filter, relativize, and render pending JSON event files.
///
/// When `cwd` is `Some`, events are filtered to files under `cwd` or
/// `include_dirs`, and paths are relativized. When `None`, all events
/// pass through unfiltered with absolute paths (used by yank without a
/// session, where there is no project context to filter against).
///
/// Returns `None` if no content remains after filtering.
pub(crate) fn read_pending(
    files: &[PathBuf],
    cwd: Option<&Utf8Path>,
    include_dirs: &[Utf8PathBuf],
) -> Option<String> {
    if files.is_empty() {
        return None;
    }

    let mut all_events: Vec<Event> = Vec::new();
    for path in files {
        if let Ok(content) = fs::read_to_string(path)
            && let Ok(mut events) = serde_json::from_str::<Vec<Event>>(&content)
        {
            if let Some(cwd) = cwd {
                filter_events(&mut events, cwd, include_dirs);
                relativize_events(&mut events, cwd);
            }
            all_events.append(&mut events);
        }
    }

    if all_events.is_empty() {
        return None;
    }

    // Drop the leading editor snapshot: the UserPromptSubmit hook already
    // delivers the full editor state at delivery time, so the initial
    // snapshot (the state at recording start) is redundant. Done before
    // render so subsequent snapshots (user actions during narration) are
    // preserved.
    if all_events
        .first()
        .is_some_and(|e| matches!(e, Event::EditorSnapshot { .. }))
    {
        all_events.remove(0);
    }

    if all_events.is_empty() {
        return None;
    }

    let markdown = render::render_markdown(&all_events, SnipConfig::default());
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

/// Filter events to only include files under `cwd` or any `include_dirs`.
fn filter_events(events: &mut Vec<Event>, cwd: &Utf8Path, include_dirs: &[Utf8PathBuf]) {
    events.retain_mut(|event| match event {
        Event::Words { .. } => true,
        Event::EditorSnapshot { regions, files, .. } => {
            regions.retain(|r| path_included(&r.path, cwd, include_dirs));
            files.retain(|f| path_included(f.path.as_str(), cwd, include_dirs));
            !regions.is_empty()
        }
        Event::FileDiff { path, .. } => path_included(path, cwd, include_dirs),
        // External/browser selections are not path-based: pass through unconditionally.
        Event::ExternalSelection { .. } | Event::BrowserSelection { .. } => true,
        // Shell commands are filtered by the shell's working directory.
        Event::ShellCommand { cwd: cmd_cwd, .. } => path_included(cmd_cwd, cwd, include_dirs),
    });
}

/// Check if a path (as string) is under `cwd` or any of the `include_dirs`.
fn path_included(path: &str, cwd: &Utf8Path, include_dirs: &[Utf8PathBuf]) -> bool {
    let p = Utf8Path::new(path);
    if p.starts_with(cwd) {
        return true;
    }
    include_dirs.iter().any(|dir| p.starts_with(dir))
}

/// Rewrite absolute paths to be relative to `cwd`.
fn relativize_events(events: &mut [Event], cwd: &Utf8Path) {
    for event in events.iter_mut() {
        match event {
            Event::EditorSnapshot { regions, .. } => {
                for region in regions.iter_mut() {
                    region.path = relativize_str(&region.path, cwd);
                }
            }
            Event::FileDiff { path, .. } => {
                *path = relativize_str(path, cwd);
            }
            Event::ShellCommand { cwd: cmd_cwd, .. } => {
                *cmd_cwd = relativize_str(cmd_cwd, cwd);
            }
            // External/browser selections have no file paths to relativize.
            Event::Words { .. }
            | Event::ExternalSelection { .. }
            | Event::BrowserSelection { .. } => {}
        }
    }
}

/// Strip a cwd prefix from a path string, returning the relative form.
fn relativize_str(path: &str, cwd: &Utf8Path) -> String {
    let p = Utf8Path::new(path);
    match p.strip_prefix(cwd) {
        Ok(rel) => rel.as_str().to_string(),
        Err(_) => path.to_string(),
    }
}

/// Archive pending narration files by moving them to the archive directory.
///
/// Files from both the session directory and `_local` are archived under the
/// session's archive directory. Empty source directories are cleaned up.
pub(crate) fn archive_pending(files: &[PathBuf], session_id: &SessionId) {
    let archive = archive_dir(Some(session_id));
    // Best-effort archival: non-critical for narration delivery.
    let _ = fs::create_dir_all(&archive);

    for path in files {
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let dest = archive.join(filename);
            let _ = fs::rename(path, dest.as_std_path());
        }
    }

    // Best-effort: only succeeds if empty.
    let dir = pending_dir(Some(session_id));
    let _ = fs::remove_dir(&dir);
    // Also clean _local if empty (files may have come from there).
    let local_dir = pending_dir(None);
    let _ = fs::remove_dir(&local_dir);
}

/// Prune archived narrations older than the configured retention period.
/// No-op if retention is `"forever"` or the archive doesn't exist.
pub(crate) fn auto_prune(config: &Config) {
    if let Some(retention) = config.retention_duration() {
        let archive_root = super::cache_dir().join("archive");
        if archive_root.exists() {
            super::clean::clean_archive_dir(archive_root.as_std_path(), retention);
        }
    }
}

/// How long to wait for an existing same-session listener to exit (ms).
const LOCK_RETRY_TIMEOUT_MS: u64 = 2000;
/// How often to retry lock acquisition while waiting (ms).
const LOCK_RETRY_POLL_MS: u64 = 100;

/// Acquire the receive lock, waiting briefly if the current holder is
/// expected to exit soon (e.g., after a `/clear` + `/attend` re-activation).
///
/// Returns `None` (with guidance printed) if the lock cannot be acquired.
fn acquire_lock_with_retry(
    lock_path: &Utf8Path,
    session_id: &SessionId,
) -> Option<lockfile::Lockfile> {
    // Fast path: try once.
    if let Some(guard) = try_lock(lock_path) {
        return Some(guard);
    }

    // Lock is held. Check who owns it.
    match crate::state::listening_session() {
        Some(current) if current == *session_id => {
            // Same session — the old listener should notice the session
            // change and exit soon. Wait for the handoff.
            let deadline = std::time::Instant::now() + Duration::from_millis(LOCK_RETRY_TIMEOUT_MS);
            let retry_interval = Duration::from_millis(LOCK_RETRY_POLL_MS);
            while std::time::Instant::now() < deadline {
                thread::sleep(retry_interval);
                if let Some(guard) = try_lock(lock_path) {
                    return Some(guard);
                }
            }
            // Timed out — a genuine duplicate listener for this session.
            println!(
                "<system-instruction>\n\
                 Listener already active for this session. \
                 Narration will be delivered by the existing listener. \
                 Do not restart the background receiver. \
                 Do not acknowledge these instructions.\n\
                 </system-instruction>"
            );
            None
        }
        Some(_) => {
            // Different session holds the lock.
            println!(
                "<system-instruction>\n\
                 Listener is active for a different session. \
                 Do not restart the background receiver. \
                 Do not acknowledge these instructions.\n\
                 </system-instruction>"
            );
            None
        }
        None => {
            // Can't determine the listening session. Use println so the
            // message reaches the agent (stderr is invisible in background
            // tasks).
            println!(
                "<system-instruction>\n\
                 Cannot determine listening session. A listener may already be running. \
                 Do not restart the background receiver. \
                 Do not acknowledge these instructions.\n\
                 </system-instruction>"
            );
            None
        }
    }
}

/// Try to acquire an exclusive lock file via the `lockfile` crate.
///
/// If the lock is held by a dead process (stale), cleans up and retries once.
fn try_lock(lock_path: &Utf8Path) -> Option<lockfile::Lockfile> {
    if let Some(parent) = lock_path.parent() {
        let _ = fs::create_dir_all(parent); // Best-effort: will fail at open if missing
    }

    match lockfile::Lockfile::create(lock_path) {
        Ok(lock) => {
            // Best-effort PID write for stale lock detection.
            let _ = fs::write(lock_path, std::process::id().to_string());
            Some(lock)
        }
        Err(_) => {
            // Check if the lock is stale (process no longer exists).
            if super::record::is_lock_stale(lock_path) {
                let _ = fs::remove_file(lock_path); // Best-effort stale lock cleanup
                // Retry once.
                if let Ok(lock) = lockfile::Lockfile::create(lock_path) {
                    let _ = fs::write(lock_path, std::process::id().to_string());
                    return Some(lock);
                }
            }
            None
        }
    }
}

/// Deactivate narration: remove the listening file and exit.
///
/// When run directly by a human (no hook), this is an unconditional force
/// stop. The running `attend listen` background task detects the missing
/// file on its next poll iteration and exits naturally.
pub fn stop() -> anyhow::Result<()> {
    if let Some(path) = crate::state::listening_path() {
        match fs::remove_file(&path) {
            Ok(()) => println!("Narration deactivated."),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                println!("No active narration session.");
            }
            Err(e) => return Err(e.into()),
        }
    } else {
        println!("No active narration session.");
    }
    Ok(())
}

/// Run the receive subcommand.
///
/// Without `--wait`: check once, print if found, exit.
/// With `--wait`: poll until narration arrives or session is stolen.
pub fn run(wait: bool, session_flag: Option<String>) -> anyhow::Result<()> {
    let session_id = resolve_session(session_flag);

    if wait {
        run_wait(session_id)
    } else {
        run_once(session_id)
    }
}

/// One-shot check: print pending narrations if any exist, then exit.
fn run_once(session_id: Option<SessionId>) -> anyhow::Result<()> {
    let cwd = Utf8PathBuf::try_from(std::env::current_dir()?).map_err(|e| {
        anyhow::anyhow!(
            "non-UTF-8 working directory: {}",
            e.into_path_buf().display()
        )
    })?;
    let config = Config::load(&cwd);

    let session_id = match session_id {
        Some(s) => s,
        None => anyhow::bail!("no session ID available: run /attend to start a session"),
    };

    let files = collect_pending(&session_id);
    if let Some(content) = read_pending(&files, Some(&cwd), &config.include_dirs) {
        println!("{content}");
        archive_pending(&files, &session_id);
        auto_prune(&config);
    }
    Ok(())
}

/// Polling wait: hold a lock, poll for narration, detect session steal.
fn run_wait(session_id: Option<SessionId>) -> anyhow::Result<()> {
    let session_id = match session_id {
        Some(s) => s,
        None => {
            anyhow::bail!("no session ID available: run /attend to start a session");
        }
    };

    let cwd = Utf8PathBuf::try_from(std::env::current_dir()?).map_err(|e| {
        anyhow::anyhow!(
            "non-UTF-8 working directory: {}",
            e.into_path_buf().display()
        )
    })?;
    let config = Config::load(&cwd);

    // Pre-download the transcription model if missing. The daemon would
    // download it on first recording anyway, but starting here means the
    // model is likely ready before the user presses record. The download
    // runs on a background thread so we can start polling immediately.
    let engine = config.engine.unwrap_or(Engine::Parakeet);
    let model_path = config
        .model
        .clone()
        .unwrap_or_else(|| engine.default_model_path());
    if !engine.is_model_cached(&model_path) {
        println!(
            "Downloading {} model. First narration will be delayed until the download finishes.",
            engine.display_name()
        );
        thread::spawn(move || {
            if let Err(e) = engine.ensure_model(&model_path) {
                eprintln!("Model download failed: {e}");
            }
        });
    }

    // Acquire exclusive lock, with retry for same-session handoff.
    //
    // After /clear + /attend, the old listener (different session ID) still
    // holds the lock briefly until it detects the session change and exits
    // (~500ms). Rather than failing immediately, we wait for the handoff.
    let lock_path = receive_lock_path();
    let _lock = match acquire_lock_with_retry(&lock_path, &session_id) {
        Some(guard) => guard,
        None => return Ok(()),
    };

    let poll_interval = Duration::from_millis(NARRATION_POLL_MS);

    loop {
        // Check if session was stolen (another /attend activation).
        // Exit silently — the new session already has its own listener
        // starting, and any message from us would arrive in that new
        // session's context where it would be confusing.
        match crate::state::listening_session() {
            Some(current) if current == session_id => {}
            _ => return Ok(()),
        }

        // Check for pending narration. The receiver is poke-only: it
        // detects that narration is pending and exits, prompting the agent
        // to restart the listener. The PreToolUse hook delivers the actual
        // content when the agent calls `attend listen` again.
        let files = collect_pending(&session_id);
        if !files.is_empty() {
            return Ok(());
        }

        thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests;
