//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.

mod audio;
mod capture;
mod clean;
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
/// # Safety
/// Calls `libc::kill(pid, 0)` which checks process existence without
/// sending a signal. This is the POSIX-specified way to probe for a process.
pub(crate) fn process_alive(pid: i32) -> bool {
    unsafe { libc::kill(pid, 0) == 0 }
}

/// Base directory for all narration state files.
fn cache_dir() -> Utf8PathBuf {
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

/// Resolve the session ID from flag, listening file, or None.
pub(crate) fn resolve_session(flag: Option<String>) -> Option<SessionId> {
    flag.map(SessionId::from)
        .or_else(crate::state::listening_session)
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

#[cfg(test)]
mod tests;
