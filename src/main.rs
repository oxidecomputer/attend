//! Read editor state (open files, selections) for agent integration.

mod agent;
mod cli;
mod editor;
mod hook;
mod state;
mod view;

use clap::Parser;

use cli::Cli;

fn main() -> anyhow::Result<()> {
    Cli::parse().run()?;
    Ok(())
}
