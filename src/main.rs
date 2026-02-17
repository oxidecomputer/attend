mod cli;
mod db;
mod format;
mod hook;
mod model;

use clap::Parser;

use cli::Cli;

fn main() -> anyhow::Result<()> {
    Cli::parse().run()?;
    Ok(())
}
