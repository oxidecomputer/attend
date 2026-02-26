//! Handler for the `meditate` subcommand.

/// Arguments for the `meditate` subcommand.
#[derive(clap::Args)]
pub struct MeditateArgs {
    /// Override polling / debounce interval in seconds.
    #[arg(long, short = 'i')]
    pub interval: Option<f64>,
}

impl MeditateArgs {
    pub fn run(self) -> anyhow::Result<()> {
        crate::watch::run(
            crate::watch::WatchMode::Silent,
            None,
            self.interval,
            &super::Format::Human,
            false,
            None,
            None,
        )
    }
}
