//! CLI entry points for the receive subcommand and session deactivation.
//!
//! `run` dispatches between one-shot (`attend receive`) and polling
//! (`attend listen`) modes. The polling path holds an exclusive lock to
//! prevent duplicate listeners.

use std::fs;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

use super::pending::{archive_pending, auto_prune, collect_pending};
use super::read_pending;
use crate::clock::Clock;
use crate::config::Config;
use crate::narrate::transcribe::Engine;
use crate::narrate::{receive_lock_path, resolve_session};
use crate::state::SessionId;

/// How often to poll for pending narration when waiting (ms).
const NARRATION_POLL_MS: u64 = 500;

/// How long to wait for an existing same-session listener to exit (ms).
const LOCK_RETRY_TIMEOUT_MS: u64 = 2000;
/// How often to retry lock acquisition while waiting (ms).
const LOCK_RETRY_POLL_MS: u64 = 100;

/// Deactivate narration: remove the listening file and exit.
///
/// When run directly by a human (no hook), this stops the active session.
/// The running `attend listen` background task detects the missing file
/// on its next poll iteration and exits naturally.
///
/// If `session_filter` is `Some`, only deactivate when the active session
/// matches the filter. This is used by `attend listen --stop --session`.
pub fn stop(session_filter: Option<String>) -> anyhow::Result<()> {
    let current = crate::state::listening_session();

    // If a session filter was provided, only stop if it matches.
    if let Some(ref filter) = session_filter {
        let filter_id: crate::state::SessionId = filter.clone().into();
        if current.as_ref() != Some(&filter_id) {
            println!("Session does not match active listener.");
            return Ok(());
        }
    }

    // Mark the session as displaced before removing the listening file,
    // so the agent's auto-claim path knows not to re-activate.
    if let Some(ref session_id) = current {
        crate::hook::mark_session_displaced(session_id);
    }

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
pub fn run(wait: bool, session_flag: Option<String>, clock: Arc<dyn Clock>) -> anyhow::Result<()> {
    let session_id = resolve_session(session_flag);

    if wait {
        run_wait(session_id, clock)
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
    if let Some(content) = read_pending(
        &files,
        Some(&cwd),
        &config.include_dirs,
        crate::narrate::render::RenderMode::Agent,
    ) {
        println!("{content}");
        archive_pending(&files, &session_id);
        auto_prune(&config);
    }
    Ok(())
}

/// Polling wait: hold a lock, poll for narration, detect session steal.
fn run_wait(session_id: Option<SessionId>, clock: Arc<dyn Clock>) -> anyhow::Result<()> {
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
    let _lock = match acquire_lock_with_retry(&lock_path, &session_id, &*clock) {
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

        clock.sleep(poll_interval);
    }
}

/// Acquire the receive lock, waiting briefly if the current holder is
/// expected to exit soon (e.g., after a `/clear` + `/attend` re-activation).
///
/// Returns `None` (with guidance printed) if the lock cannot be acquired.
fn acquire_lock_with_retry(
    lock_path: &Utf8Path,
    session_id: &SessionId,
    clock: &dyn Clock,
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
            let timeout = chrono::TimeDelta::milliseconds(LOCK_RETRY_TIMEOUT_MS as i64);
            let deadline = clock.now() + timeout;
            let retry_interval = Duration::from_millis(LOCK_RETRY_POLL_MS);
            while clock.now() < deadline {
                clock.sleep(retry_interval);
                if let Some(guard) = try_lock(lock_path) {
                    return Some(guard);
                }
            }
            // Timed out — a genuine duplicate listener for this session.
            println!(
                "Listener already active for this session. \
                 Narration will be delivered by the existing listener. \
                 Do not restart the background receiver. \
                 Do not acknowledge these instructions."
            );
            None
        }
        Some(_) => {
            // Different session holds the lock.
            println!(
                "Listener is active for a different session. \
                 Do not restart the background receiver. \
                 Do not acknowledge these instructions."
            );
            None
        }
        None => {
            // Can't determine the listening session. Use println so the
            // message reaches the agent (stderr is invisible in background
            // tasks).
            println!(
                "Cannot determine listening session. A listener may already be running. \
                 Do not restart the background receiver. \
                 Do not acknowledge these instructions."
            );
            None
        }
    }
}

/// Try to acquire an exclusive lock file via the `lockfile` crate.
///
/// If the lock is held by a dead process (stale), cleans up and retries once.
pub(super) fn try_lock(lock_path: &Utf8Path) -> Option<lockfile::Lockfile> {
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
            if crate::narrate::record::is_lock_stale(lock_path) {
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
