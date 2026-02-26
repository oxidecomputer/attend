//! Handler for the `completions` subcommand.

use clap::CommandFactory;

/// Arguments for the `completions` subcommand.
#[derive(clap::Args)]
pub struct CompletionsArgs {
    /// Shell to generate completions for.
    pub shell: clap_complete::Shell,
}

impl CompletionsArgs {
    pub fn run(self) -> anyhow::Result<()> {
        clap_complete::generate(
            self.shell,
            &mut super::Cli::command(),
            "attend",
            &mut std::io::stdout(),
        );
        Ok(())
    }
}
