//! Handler for the `listen` subcommand.

/// Arguments for the `listen` subcommand.
#[derive(clap::Args)]
pub struct ListenArgs {
    /// Check once and exit instead of waiting.
    #[arg(long)]
    pub check: bool,

    /// Session ID (defaults to listening file).
    #[arg(long)]
    pub session: Option<String>,

    /// Deactivate narration: remove the listening file and exit.
    /// With --session, only deactivate if the active session matches.
    #[arg(long, conflicts_with = "check")]
    pub stop: bool,
}

impl ListenArgs {
    pub fn run(self) -> anyhow::Result<()> {
        if self.stop {
            crate::narrate::receive::stop(self.session)
        } else {
            // check → one-shot (old `receive` without --wait)
            // default → wait (old `receive --wait`)
            crate::narrate::receive::run(!self.check, self.session)
        }
    }
}
