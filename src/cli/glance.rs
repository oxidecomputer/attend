//! Handler for the `glance` subcommand.

use camino::Utf8PathBuf;

use super::Format;

/// Arguments for the `glance` subcommand.
#[derive(clap::Args)]
pub struct GlanceArgs {
    /// Resolve paths relative to this directory and show relative paths.
    #[arg(long, short)]
    pub dir: Option<Utf8PathBuf>,

    /// Output format.
    #[arg(long, short, default_value = "human")]
    pub format: Format,

    /// Continuously watch editor state.
    #[arg(long, short)]
    pub watch: bool,

    /// Override polling / debounce interval in seconds.
    #[arg(long, short = 'i')]
    pub interval: Option<f64>,
}

impl GlanceArgs {
    /// Run the glance subcommand (one-shot or watch mode).
    pub fn run(self) -> anyhow::Result<()> {
        if self.watch {
            crate::watch::run(
                crate::watch::WatchMode::Compact,
                self.dir.as_deref(),
                self.interval,
                &self.format,
                false,
                None,
                None,
                crate::clock::process_clock().for_thread(),
            )
        } else {
            if let Some(state) = crate::state::EditorState::current(self.dir.as_deref(), &[])? {
                match self.format {
                    Format::Human => println!("{state}"),
                    Format::Json => {
                        let payload = crate::state::CompactPayload::from_state(&state);
                        let wrapped = crate::util::Timestamped::at(chrono::Utc::now(), payload);
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&wrapped)
                                .expect("serialization of known type")
                        );
                    }
                }
            }
            Ok(())
        }
    }
}
