//! Read editor state (open files, selections) for agent integration.

#[cfg(not(unix))]
compile_error!("attend requires a Unix platform (macOS or Linux)");

#[cfg(target_os = "macos")]
#[macro_use]
extern crate objc;

mod agent;
mod browser;
mod cli;
mod config;
mod editor;
mod hook;
mod narrate;
mod state;
mod terminal;
mod util;
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
