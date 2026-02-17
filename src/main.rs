//! Read Zed editor state (open files, selections, terminals) for agent integration.

mod cli;
mod db;
mod hook;
mod model;

use clap::Parser;

use cli::Cli;

fn main() -> anyhow::Result<()> {
    Cli::parse().run()?;
    Ok(())
}
