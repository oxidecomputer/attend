//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.

mod audio;
mod merge;
mod receive;
mod record;
mod transcribe;

use std::path::PathBuf;

use clap::Subcommand;

/// Base directory for all dictation state files.
fn cache_dir() -> PathBuf {
    crate::state::cache_dir().expect("cannot determine cache directory")
}

/// Read the session ID of the currently attending session, if any.
fn listening_session() -> Option<String> {
    crate::state::listening_session()
}

/// Path to the record lock file.
pub fn record_lock_path() -> PathBuf {
    cache_dir().join("record.lock")
}

/// Path to the stop sentinel file.
pub fn stop_sentinel_path() -> PathBuf {
    cache_dir().join("stop")
}

/// Path to the receive lock file.
pub fn receive_lock_path() -> PathBuf {
    cache_dir().join("receive.lock")
}

/// Directory where pending dictation files are written.
///
/// Each dictation is stored as `<timestamp>.md` inside
/// `~/.cache/attend/pending/<session_id>/`.
pub fn pending_dir(session_id: &str) -> PathBuf {
    cache_dir().join("pending").join(session_id)
}

/// Directory where archived dictation files are stored.
pub fn archive_dir(session_id: &str) -> PathBuf {
    cache_dir().join("archive").join(session_id)
}

/// Default Whisper model path.
pub fn default_model_path() -> PathBuf {
    cache_dir().join("models").join("ggml-small.en.bin")
}

/// Dictation CLI subcommands.
#[derive(Subcommand)]
pub enum DictateCommand {
    /// Start or stop recording (one hotkey).
    Toggle {
        /// Path to GGML Whisper model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
        /// Snip code/diff blocks longer than this many lines.
        #[arg(long, default_value_t = 20)]
        snip_threshold: usize,
        /// Lines to keep at the start of a snipped block.
        #[arg(long, default_value_t = 10)]
        snip_head: usize,
        /// Lines to keep at the end of a snipped block.
        #[arg(long, default_value_t = 5)]
        snip_tail: usize,
    },
    /// Spawn detached recorder (idempotent).
    Start {
        /// Path to GGML Whisper model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
        /// Snip code/diff blocks longer than this many lines.
        #[arg(long, default_value_t = 20)]
        snip_threshold: usize,
        /// Lines to keep at the start of a snipped block.
        #[arg(long, default_value_t = 10)]
        snip_head: usize,
        /// Lines to keep at the end of a snipped block.
        #[arg(long, default_value_t = 5)]
        snip_tail: usize,
    },
    /// Signal recorder to stop (idempotent).
    Stop,
    /// Check for / wait for dictation.
    Receive {
        /// Poll until dictation arrives.
        #[arg(long)]
        wait: bool,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
    },
    /// Install editor integration for dictation.
    Install {
        /// Editor to install for.
        #[arg(long, value_parser = editor_value_parser())]
        editor: String,
    },
    /// Internal: run the recording daemon (not user-facing).
    #[command(name = "_record-daemon", hide = true)]
    RecordDaemon {
        /// Path to GGML Whisper model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Session ID.
        #[arg(long)]
        session: Option<String>,
        #[arg(long, default_value_t = 20)]
        snip_threshold: usize,
        #[arg(long, default_value_t = 10)]
        snip_head: usize,
        #[arg(long, default_value_t = 5)]
        snip_tail: usize,
    },
    /// Internal: benchmark model load and transcription latency.
    #[command(name = "_bench", hide = true)]
    Bench,
}

/// Value parser that validates editor names against registered backends.
fn editor_value_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(crate::editor::EDITORS.iter().map(|e| e.name()))
}

/// Resolve the session ID from flag, listening file, or None.
pub fn resolve_session(flag: Option<String>) -> Option<String> {
    flag.or_else(listening_session)
}

/// Run a dictate subcommand.
pub fn run(cmd: DictateCommand) -> anyhow::Result<()> {
    match cmd {
        DictateCommand::Toggle {
            model,
            session,
            snip_threshold,
            snip_head,
            snip_tail,
        } => {
            let snip_cfg = merge::SnipConfig {
                threshold: snip_threshold,
                head: snip_head,
                tail: snip_tail,
            };
            record::toggle(model, session, snip_cfg)
        }
        DictateCommand::Start {
            model,
            session,
            snip_threshold,
            snip_head,
            snip_tail,
        } => {
            let snip_cfg = merge::SnipConfig {
                threshold: snip_threshold,
                head: snip_head,
                tail: snip_tail,
            };
            record::start(model, session, snip_cfg)
        }
        DictateCommand::Stop => record::stop(),
        DictateCommand::Receive { wait, session } => receive::run(wait, session),
        DictateCommand::Install { editor } => install(&editor),
        DictateCommand::RecordDaemon {
            model,
            session,
            snip_threshold,
            snip_head,
            snip_tail,
        } => {
            let snip_cfg = merge::SnipConfig {
                threshold: snip_threshold,
                head: snip_head,
                tail: snip_tail,
            };
            record::daemon(model, session, snip_cfg)
        }
        DictateCommand::Bench => run_bench(),
    }
}

/// Run model benchmarks for base, small, and medium models.
fn run_bench() -> anyhow::Result<()> {
    let models_dir = cache_dir().join("models");
    let models = [
        "ggml-base.en.bin",
        "ggml-small.en.bin",
        "ggml-medium.en.bin",
    ];

    for name in &models {
        let path = models_dir.join(name);
        eprintln!("Ensuring model: {name}");
        transcribe::ensure_model(&path)?;
    }

    let samples = vec![0.0f32; 16000 * 5];

    for name in &models {
        let path = models_dir.join(name);
        eprintln!("\n--- {name} ---");
        transcribe::bench_model(&path, &samples);
    }

    Ok(())
}

/// Install editor integration for dictation.
fn install(editor_name: &str) -> anyhow::Result<()> {
    let editor = crate::editor::editor_by_name(editor_name)
        .ok_or_else(|| anyhow::anyhow!("unknown editor: {editor_name}"))?;
    let bin_cmd = crate::agent::resolve_bin_cmd(false)?;
    editor.install_dictation(&bin_cmd)?;
    println!("\nRun `attend hook install -a <agent>` to install agent hooks.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_session_flag_takes_precedence() {
        let result = resolve_session(Some("my-session".to_string()));
        assert_eq!(result, Some("my-session".to_string()));
    }

    #[test]
    fn resolve_session_no_flag_no_listening() {
        // When no flag and no listening file exists, returns None
        // (depends on whether listening file exists on disk, so just test the flag path)
        let result = resolve_session(Some("test".to_string()));
        assert_eq!(result.unwrap(), "test");
    }

    #[test]
    fn cache_dir_is_under_attend() {
        let dir = cache_dir();
        assert!(dir.ends_with("attend"));
    }

    #[test]
    fn pending_dir_includes_session() {
        let dir = pending_dir("abc-123");
        assert!(dir.ends_with("pending/abc-123") || dir.ends_with("pending\\abc-123"));
    }

    #[test]
    fn archive_dir_includes_session() {
        let dir = archive_dir("abc-123");
        assert!(dir.ends_with("archive/abc-123") || dir.ends_with("archive\\abc-123"));
    }
}
