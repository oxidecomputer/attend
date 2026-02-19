use std::path::PathBuf;

use clap::Subcommand;

use crate::dictate::transcribe::Engine;

/// Value parser that validates editor names against registered backends.
fn editor_value_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(crate::editor::EDITORS.iter().map(|e| e.name()))
}

/// Dictation CLI subcommands.
#[derive(Subcommand)]
pub enum DictateCommand {
    /// Start or stop recording (one hotkey).
    Toggle {
        /// Transcription engine.
        #[arg(long, value_enum, default_value_t = Engine::Parakeet)]
        engine: Engine,
        /// Path to model file or directory.
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
        /// Transcription engine.
        #[arg(long, value_enum, default_value_t = Engine::Parakeet)]
        engine: Engine,
        /// Path to model file or directory.
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
        /// Transcription engine.
        #[arg(long, value_enum, default_value_t = Engine::Parakeet)]
        engine: Engine,
        /// Path to model file or directory.
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

impl DictateCommand {
    /// Run a dictate subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        use crate::dictate::{merge::SnipConfig, receive, record};

        match self {
            DictateCommand::Toggle {
                engine,
                model,
                session,
                snip_threshold,
                snip_head,
                snip_tail,
            } => {
                let snip_cfg = SnipConfig {
                    threshold: snip_threshold,
                    head: snip_head,
                    tail: snip_tail,
                };
                record::toggle(engine, model, session, snip_cfg)
            }
            DictateCommand::Start {
                engine,
                model,
                session,
                snip_threshold,
                snip_head,
                snip_tail,
            } => {
                let snip_cfg = SnipConfig {
                    threshold: snip_threshold,
                    head: snip_head,
                    tail: snip_tail,
                };
                record::start(engine, model, session, snip_cfg)
            }
            DictateCommand::Stop => record::stop(),
            DictateCommand::Receive { wait, session } => receive::run(wait, session),
            DictateCommand::Install { editor } => install(&editor),
            DictateCommand::RecordDaemon {
                engine,
                model,
                session,
                snip_threshold,
                snip_head,
                snip_tail,
            } => {
                let snip_cfg = SnipConfig {
                    threshold: snip_threshold,
                    head: snip_head,
                    tail: snip_tail,
                };
                record::daemon(engine, model, session, snip_cfg)
            }
            DictateCommand::Bench => crate::dictate::bench(),
        }
    }
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
