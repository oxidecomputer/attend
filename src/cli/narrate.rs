use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::narrate::transcribe::Engine;

/// Parse a human-friendly duration string (e.g. "7d", "24h", "30days").
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    humantime::parse_duration(s).map_err(|e| e.to_string())
}

/// Shared recording arguments for toggle / start / daemon.
#[derive(Args, Clone)]
pub struct RecordingArgs {
    /// Transcription engine.
    #[arg(long, value_enum, default_value_t = Engine::Parakeet)]
    engine: Engine,
    /// Path to model file or directory (auto-downloaded if omitted).
    #[arg(long)]
    model: Option<PathBuf>,
    /// Session to deliver narration to (defaults to the active `attend hook` session).
    #[arg(long)]
    session: Option<String>,
}

/// Narration CLI subcommands.
#[derive(Subcommand)]
pub enum NarrateCommand {
    /// Start or stop recording (one hotkey).
    Toggle {
        #[command(flatten)]
        args: RecordingArgs,
    },
    /// Submit current narration and keep recording.
    ///
    /// If not recording, starts recording (like toggle).
    /// If recording, flushes the current audio for transcription
    /// while continuing to record.
    Flush {
        #[command(flatten)]
        args: RecordingArgs,
    },
    /// Spawn detached recorder (idempotent).
    #[command(hide = true)]
    Start {
        #[command(flatten)]
        args: RecordingArgs,
    },
    /// Signal recorder to stop (idempotent).
    Stop,
    /// Show recording and system status.
    Status,
    /// Remove old archived narration files.
    Clean {
        /// Remove archives older than this duration (e.g. "7d", "24h", "30days").
        #[arg(long, default_value = "7d", value_parser = parse_duration)]
        older_than: std::time::Duration,
    },
    /// Internal: run the recording daemon (not user-facing).
    #[command(name = "_record-daemon", hide = true)]
    RecordDaemon {
        #[command(flatten)]
        args: RecordingArgs,
    },
    /// Internal: benchmark model load and transcription latency.
    #[command(name = "_bench", hide = true)]
    Bench,
}

impl NarrateCommand {
    /// Run a narrate subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        use crate::narrate::record;

        match self {
            NarrateCommand::Toggle { args } => {
                record::toggle(args.engine, args.model, args.session)
            }
            NarrateCommand::Flush { args } => record::flush(args.engine, args.model, args.session),
            NarrateCommand::Start { args } => record::start(args.engine, args.model, args.session),
            NarrateCommand::Stop => record::stop(),
            NarrateCommand::Status => crate::narrate::status(),
            NarrateCommand::Clean { older_than } => crate::narrate::clean(older_than),
            NarrateCommand::RecordDaemon { args } => {
                record::daemon(args.engine, args.model, args.session)
            }
            NarrateCommand::Bench => crate::narrate::bench(),
        }
    }
}
