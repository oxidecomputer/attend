use std::path::PathBuf;

use clap::{Args, Subcommand};

use crate::dictate::merge::SnipConfig;
use crate::dictate::transcribe::Engine;

/// Shared recording arguments for toggle / start / daemon.
#[derive(Args, Clone)]
pub struct RecordingArgs {
    /// Transcription engine.
    #[arg(long, value_enum, default_value_t = Engine::Parakeet)]
    engine: Engine,
    /// Path to model file or directory (auto-downloaded if omitted).
    #[arg(long)]
    model: Option<PathBuf>,
    /// Session to deliver dictation to (defaults to the active `attend hook` session).
    #[arg(long)]
    session: Option<String>,
    #[command(flatten)]
    snip: SnipArgs,
}

/// Snip configuration for large code/diff blocks.
#[derive(Args, Clone)]
pub struct SnipArgs {
    /// Snip code/diff blocks longer than this many lines.
    #[arg(long, default_value_t = 20, help_heading = "Advanced")]
    snip_threshold: usize,
    /// Lines to keep at the start of a snipped block.
    #[arg(long, default_value_t = 10, help_heading = "Advanced")]
    snip_head: usize,
    /// Lines to keep at the end of a snipped block.
    #[arg(long, default_value_t = 5, help_heading = "Advanced")]
    snip_tail: usize,
}

impl SnipArgs {
    fn into_config(self) -> SnipConfig {
        SnipConfig {
            threshold: self.snip_threshold,
            head: self.snip_head,
            tail: self.snip_tail,
        }
    }
}

/// Dictation CLI subcommands.
#[derive(Subcommand)]
pub enum DictateCommand {
    /// Start or stop recording (one hotkey).
    Toggle {
        #[command(flatten)]
        args: RecordingArgs,
    },
    /// Submit current dictation and keep recording.
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
    /// Check for / wait for dictation.
    Receive {
        /// Poll until dictation arrives.
        #[arg(long)]
        wait: bool,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
    },
    /// Show recording and system status.
    Status,
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

impl DictateCommand {
    /// Run a dictate subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        use crate::dictate::record;

        match self {
            DictateCommand::Toggle { args } => {
                record::toggle(args.engine, args.model, args.session, args.snip.into_config())
            }
            DictateCommand::Flush { args } => {
                record::flush(args.engine, args.model, args.session, args.snip.into_config())
            }
            DictateCommand::Start { args } => {
                record::start(args.engine, args.model, args.session, args.snip.into_config())
            }
            DictateCommand::Stop => record::stop(),
            DictateCommand::Receive { wait, session } => {
                crate::dictate::receive::run(wait, session)
            }
            DictateCommand::Status => crate::dictate::status(),
            DictateCommand::RecordDaemon { args } => {
                record::daemon(args.engine, args.model, args.session, args.snip.into_config())
            }
            DictateCommand::Bench => crate::dictate::bench(),
        }
    }
}
