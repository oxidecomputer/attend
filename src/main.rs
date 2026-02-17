//! Read editor state (open files, selections, terminals) for agent integration.

mod agent;
mod cli;
mod editor;
mod hook;
mod state;

use clap::Parser;

use cli::Cli;

fn main() -> anyhow::Result<()> {
    Cli::parse().run()?;
    Ok(())
}
