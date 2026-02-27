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
    /// Pause or resume narration.
    Pause,
    /// Stop narration and copy to clipboard.
    Yank,
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

/// Run model benchmarks for all engines and model variants.
fn bench() -> anyhow::Result<()> {
    use crate::narrate::transcribe::Engine;

    let models_dir = crate::narrate::cache_dir().join("models");
    let samples = vec![0.0f32; 16000 * 5];

    for engine in &[Engine::Whisper, Engine::Parakeet] {
        for name in engine.model_names() {
            let path = models_dir.join(name);
            tracing::info!("Ensuring model: {name}");
            engine.preload(&path)?;
            tracing::info!("--- {name} ---");
            let mut transcriber = engine.ensure_and_load(&path)?;
            transcriber.bench(&samples);
        }
    }

    Ok(())
}

impl NarrateCommand {
    /// Run a narrate subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        use crate::narrate::record;

        let clock = crate::clock::process_clock();
        match self {
            NarrateCommand::Toggle => record::toggle(&*clock),
            NarrateCommand::Start => record::start(&*clock),
            NarrateCommand::Stop => record::stop(&*clock),
            NarrateCommand::Pause => record::pause(),
            NarrateCommand::Yank => record::yank(&*clock),
            NarrateCommand::Status => crate::narrate::status(),
            NarrateCommand::Clean { older_than } => crate::narrate::clean(older_than),
            NarrateCommand::RecordDaemon => record::daemon(),
            NarrateCommand::Bench => bench(),
        }
    }
}
