//! Read editor state (open files, selections) for agent integration.

mod agent;
mod cli;
mod config;
mod dictate;
mod editor;
mod hook;
mod json;
mod state;
mod view;
mod watch;

use clap::Parser;

use cli::Cli;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    Cli::parse().run()?;
    Ok(())
}
