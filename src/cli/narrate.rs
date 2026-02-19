use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::narrate::transcribe::Engine;

/// Parse a human-friendly duration string (e.g. "7d", "24h", "30days").
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    humantime::parse_duration(s).map_err(|e| e.to_string())
}

/// Hidden args forwarded to the recording daemon.
#[derive(Args, Clone)]
pub(crate) struct DaemonArgs {
    /// Transcription engine.
    #[arg(long, value_enum, default_value_t = Engine::Parakeet)]
    engine: Engine,
    /// Path to model file or directory.
    #[arg(long)]
    model: Option<PathBuf>,
    /// Session to deliver narration to.
    #[arg(long)]
    session: Option<String>,
}

/// Narration CLI subcommands.
#[derive(Subcommand)]
pub enum NarrateCommand {
    /// Start or stop recording (one hotkey).
    Toggle {
        /// Session to deliver narration to (defaults to the active `attend hook` session).
        #[arg(long)]
        session: Option<String>,
    },
    /// Submit current narration and keep recording.
    ///
    /// If not recording, starts recording (like toggle).
    /// If recording, flushes the current audio for transcription
    /// while continuing to record.
    Flush {
        /// Session to deliver narration to (defaults to the active `attend hook` session).
        #[arg(long)]
        session: Option<String>,
    },
    /// Spawn detached recorder (idempotent).
    #[command(hide = true)]
    Start {
        /// Session to deliver narration to (defaults to the active `attend hook` session).
        #[arg(long)]
        session: Option<String>,
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
        args: DaemonArgs,
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
            NarrateCommand::Toggle { session } => record::toggle(session),
            NarrateCommand::Flush { session } => record::flush(session),
            NarrateCommand::Start { session } => record::start(session),
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
