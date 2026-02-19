//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.

mod audio;
pub(crate) mod merge;
pub(crate) mod receive;
pub(crate) mod record;
pub(crate) mod transcribe;

use std::path::PathBuf;

/// Base directory for all dictation state files.
fn cache_dir() -> PathBuf {
    crate::state::cache_dir().expect("cannot determine cache directory")
}

/// Read the session ID of the currently attending session, if any.
fn listening_session() -> Option<String> {
    crate::state::listening_session()
}

/// Path to the record lock file.
pub(crate) fn record_lock_path() -> PathBuf {
    cache_dir().join("record.lock")
}

/// Path to the stop sentinel file.
pub(crate) fn stop_sentinel_path() -> PathBuf {
    cache_dir().join("stop")
}

/// Path to the receive lock file.
pub(crate) fn receive_lock_path() -> PathBuf {
    cache_dir().join("receive.lock")
}

/// Directory where pending dictation files are written.
///
/// Each dictation is stored as `<timestamp>.md` inside
/// `~/.cache/attend/pending/<session_id>/`.
pub(crate) fn pending_dir(session_id: &str) -> PathBuf {
    cache_dir().join("pending").join(session_id)
}

/// Directory where archived dictation files are stored.
pub(crate) fn archive_dir(session_id: &str) -> PathBuf {
    cache_dir().join("archive").join(session_id)
}

/// Resolve the session ID from flag, listening file, or None.
pub(crate) fn resolve_session(flag: Option<String>) -> Option<String> {
    flag.or_else(listening_session)
}

/// Run model benchmarks for all engines and model variants.
pub(crate) fn bench() -> anyhow::Result<()> {
    use transcribe::Engine;

    let models_dir = cache_dir().join("models");
    let samples = vec![0.0f32; 16000 * 5];

    for engine in &[Engine::Whisper, Engine::Parakeet] {
        for name in engine.model_names() {
            let path = models_dir.join(name);
            eprintln!("Ensuring model: {name}");
            engine.preload(&path)?; // ensure + load to verify download
            eprintln!("\n--- {name} ---");
            let mut transcriber = engine.ensure_and_load(&path)?;
            transcriber.bench(&samples);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
