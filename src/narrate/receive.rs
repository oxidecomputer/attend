//! Check for and deliver pending narration files to Claude Code.
//!
//! Narration files are stored as individual timestamped JSON files in
//! `<cache_dir>/attend/pending/<session_id>/` (platform cache directory). Each file contains a
//! `Vec<Event>` with absolute paths. On receive, events are filtered to
//! the current project directory (and any configured `include_dirs`),
//! paths are relativized, and the result is rendered as markdown wrapped
//! in `<narration>` tags.

use std::fs;
use std::path::PathBuf;
use std::thread;
use std::time::Duration;

use camino::{Utf8Path, Utf8PathBuf};

use super::merge::Event;
use super::render::{self, SnipConfig};
use super::{archive_dir, cache_dir, pending_dir, receive_lock_path, resolve_session};
use crate::config::Config;
use crate::state::SessionId;

/// How often to poll for pending narration when waiting (ms).
const NARRATION_POLL_MS: u64 = 500;

/// Re-dispatch instruction appended to output when listening.
const REDISPATCH_MSG: &str =
    "\n[Run `attend listen` in the background to wait for the next narration.]";

/// Collect all pending narration files for a session, sorted by filename (timestamp).
pub(crate) fn collect_pending(session_id: &SessionId) -> Vec<PathBuf> {
    let dir = pending_dir(session_id);
    let Ok(entries) = fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();

    files.sort();
    files
}

/// Deserialize, filter, relativize, and render pending JSON event files.
///
/// Returns `None` if no content remains after filtering.
pub(crate) fn read_pending(
    files: &[PathBuf],
    cwd: &Utf8Path,
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
            filter_events(&mut events, cwd, include_dirs);
            relativize_events(&mut events, cwd);
            all_events.append(&mut events);
        }
    }

    if all_events.is_empty() {
        return None;
    }

    let markdown = render::render_markdown(&all_events, SnipConfig::default());
    let trimmed = markdown.trim();
    if trimmed.is_empty() {
        return None;
    }

    Some(format!("<narration>\n{trimmed}\n</narration>"))
}

/// Filter events to only include files under `cwd` or any `include_dirs`.
fn filter_events(events: &mut Vec<Event>, cwd: &Utf8Path, include_dirs: &[Utf8PathBuf]) {
    events.retain_mut(|event| match event {
        Event::Words { .. } => true,
        Event::EditorSnapshot {
            rendered, files, ..
        } => {
            rendered.retain(|r| path_included(&r.path, cwd, include_dirs));
            files.retain(|f| path_included(f.path.as_str(), cwd, include_dirs));
            !rendered.is_empty()
        }
        Event::FileDiff { path, .. } => path_included(path, cwd, include_dirs),
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
            Event::EditorSnapshot { rendered, .. } => {
                for file in rendered.iter_mut() {
                    file.path = relativize_str(&file.path, cwd);
                }
            }
            Event::FileDiff { path, .. } => {
                *path = relativize_str(path, cwd);
            }
            Event::Words { .. } => {}
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
pub(crate) fn archive_pending(files: &[PathBuf], session_id: &SessionId) {
    let archive = archive_dir(session_id);
    let _ = fs::create_dir_all(&archive);

    for path in files {
        if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
            let dest = archive.join(filename);
            let _ = fs::rename(path, dest.as_std_path());
        }
    }

    // Clean up the pending directory if empty.
    let dir = pending_dir(session_id);
    let _ = fs::remove_dir(&dir); // only succeeds if empty
}

/// Try to acquire an exclusive lock file. Returns the path on success.
fn try_lock(lock_path: &Utf8Path) -> Option<LockGuard> {
    if let Some(parent) = lock_path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Use O_CREAT | O_EXCL for atomic creation
    match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(lock_path)
    {
        Ok(_) => {
            // Write our PID for debugging
            let _ = fs::write(lock_path, std::process::id().to_string());
            Some(LockGuard {
                path: lock_path.to_path_buf(),
            })
        }
        Err(_) => {
            // Check if the lock is stale (process no longer exists)
            if let Ok(content) = fs::read_to_string(lock_path)
                && let Ok(pid) = content.trim().parse::<u32>()
                && !super::process_alive(pid as i32)
            {
                let _ = fs::remove_file(lock_path);
                return try_lock(lock_path);
            }
            None
        }
    }
}

/// RAII guard that removes the lock file on drop.
struct LockGuard {
    path: Utf8PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
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
        None => {
            // No session — try unsuffixed fallback
            let fallback = cache_dir().join("narration.json");
            if fallback.exists()
                && let Ok(content) = fs::read_to_string(&fallback)
            {
                if let Ok(mut events) = serde_json::from_str::<Vec<Event>>(&content) {
                    filter_events(&mut events, &cwd, &config.include_dirs);
                    relativize_events(&mut events, &cwd);
                    let markdown = render::render_markdown(&events, SnipConfig::default());
                    let trimmed = markdown.trim();
                    if !trimmed.is_empty() {
                        print!("<narration>\n{trimmed}\n</narration>");
                        let _ = fs::remove_file(&fallback);
                        return Ok(());
                    }
                }
                let _ = fs::remove_file(&fallback);
            }
            std::process::exit(1);
        }
    };

    let files = collect_pending(&session_id);
    match read_pending(&files, &cwd, &config.include_dirs) {
        Some(content) => {
            print!("{content}");
            archive_pending(&files, &session_id);
            Ok(())
        }
        None => std::process::exit(1),
    }
}

/// Polling wait: hold a lock, poll for narration, detect session steal.
fn run_wait(session_id: Option<SessionId>) -> anyhow::Result<()> {
    let session_id = match session_id {
        Some(s) => s,
        None => {
            anyhow::bail!("no session ID available (use --session or run /attend first)");
        }
    };

    let cwd = Utf8PathBuf::try_from(std::env::current_dir()?).map_err(|e| {
        anyhow::anyhow!(
            "non-UTF-8 working directory: {}",
            e.into_path_buf().display()
        )
    })?;
    let config = Config::load(&cwd);

    // Acquire exclusive lock
    let lock_path = receive_lock_path();
    let _lock = match try_lock(&lock_path) {
        Some(guard) => guard,
        None => {
            // Another listener is already running
            eprintln!("Listener already running. Use `attend narrate status` to check.");
            std::process::exit(0);
        }
    };

    let poll_interval = Duration::from_millis(NARRATION_POLL_MS);

    loop {
        // Check if session was stolen
        match crate::state::listening_session() {
            Some(current) if current == session_id => {}
            _ => {
                println!(
                    "Narration was transferred to a session with another agent. \
                     Do not restart the background receiver. \
                     If the user wants narration in this session, they will type /attend."
                );
                return Ok(());
            }
        }

        // Check for pending narration
        let files = collect_pending(&session_id);
        if let Some(content) = read_pending(&files, &cwd, &config.include_dirs) {
            print!("{content}");
            println!("{REDISPATCH_MSG}");
            archive_pending(&files, &session_id);
            return Ok(());
        }

        thread::sleep(poll_interval);
    }
}

#[cfg(test)]
mod tests;
