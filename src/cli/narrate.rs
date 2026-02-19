use clap::Subcommand;

/// Parse a human-friendly duration string (e.g. "7d", "24h", "30days").
fn parse_duration(s: &str) -> Result<std::time::Duration, String> {
    humantime::parse_duration(s).map_err(|e| e.to_string())
}

/// Narration CLI subcommands.
#[derive(Subcommand)]
pub enum NarrateCommand {
    /// Start or stop narration.
    Toggle,
    /// Start narration, or send current narration and keep recording.
    Start,
    /// Stop narration.
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
    RecordDaemon,
    /// Internal: benchmark model load and transcription latency.
    #[command(name = "_bench", hide = true)]
    Bench,
}

impl NarrateCommand {
    /// Run a narrate subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        use crate::narrate::record;

        match self {
            NarrateCommand::Toggle => record::toggle(),
            NarrateCommand::Start => record::start(),
            NarrateCommand::Stop => record::stop(),
            NarrateCommand::Status => crate::narrate::status(),
            NarrateCommand::Clean { older_than } => crate::narrate::clean(older_than),
            NarrateCommand::RecordDaemon => record::daemon(),
            NarrateCommand::Bench => crate::narrate::bench(),
        }
    }
}
