mod dictate;
mod hook;

pub use dictate::DictateCommand;
pub use hook::Hook;

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

/// Top-level CLI definition.
#[derive(Parser)]
#[command(
    name = "attend",
    about = "Read editor state for AI coding agents.",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Resolve paths relative to this directory and show relative paths.
    #[arg(long, short)]
    pub dir: Option<PathBuf>,

    /// Output format (only valid without a subcommand).
    #[arg(long, short, default_value = "human")]
    pub format: Format,
}

/// Output format for the default command.
#[derive(Clone, ValueEnum)]
pub enum Format {
    /// Human-readable text.
    Human,
    /// Pretty-printed JSON.
    Json,
}

/// Display mode for the watch subcommand.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum WatchMode {
    /// Daemon: continuously update cache, no output.
    Silent,
    /// Live compact output (paths + positions).
    Compact,
    /// Live view output (file content with markers).
    View,
}

/// Continuously watch editor state.
#[derive(Args)]
pub struct Watch {
    /// Display mode.
    #[arg(default_value = "silent")]
    pub mode: WatchMode,

    /// Override polling / debounce interval in seconds.
    #[arg(long, short = 'i')]
    pub interval: Option<f64>,

    /// Show entire file contents (view mode only).
    #[arg(long, conflicts_with_all = ["before", "after"])]
    pub full: bool,

    /// Context lines before (view mode only).
    #[arg(long, short = 'B')]
    pub before: Option<usize>,

    /// Context lines after (view mode only).
    #[arg(long, short = 'A')]
    pub after: Option<usize>,

    /// Output format (compact and view modes only).
    #[arg(long, short, default_value = "human")]
    pub format: Format,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Hook mode for agent integration.
    #[command(subcommand)]
    Hook(Hook),
    /// Continuously watch editor state.
    Watch(Watch),
    /// Voice-driven prompt composition.
    #[command(subcommand)]
    Dictate(DictateCommand),
    /// Show file content at editor selections.
    View {
        /// Resolve paths relative to this directory and show relative paths.
        #[arg(long, short)]
        dir: Option<PathBuf>,

        /// Output format.
        #[arg(long, short, default_value = "human")]
        format: Format,

        /// Show entire file contents with highlights inline.
        #[arg(long, conflicts_with_all = ["before", "after"])]
        full: bool,

        /// Context lines before each excerpt.
        #[arg(long, short = 'B')]
        before: Option<usize>,

        /// Context lines after each excerpt.
        #[arg(long, short = 'A')]
        after: Option<usize>,

        /// File paths and positions in compact format (same as default output).
        /// E.g.: src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
}

impl Cli {
    /// Run the CLI: dispatch to subcommand or print editor state.
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            Some(command) => {
                if !matches!(self.format, Format::Human) {
                    anyhow::bail!(
                        "--format is only valid without a subcommand (use subcommand's own --format)"
                    );
                }
                command.run(self.dir)?;
            }
            None => {
                if let Some(state) = crate::state::EditorState::current(self.dir.as_deref(), &[])? {
                    match self.format {
                        Format::Human => println!("{state}"),
                        Format::Json => {
                            let payload = crate::json::CompactPayload::from_state(&state);
                            let wrapped = crate::json::Timestamped::now(payload);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&wrapped).unwrap_or_default()
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }
}

impl Command {
    /// Execute a subcommand.
    pub fn run(self, cwd: Option<PathBuf>) -> anyhow::Result<()> {
        match self {
            Command::Watch(watch) => crate::watch::run(&watch, cwd.as_deref()),
            Command::Dictate(cmd) => cmd.run(),
            Command::Hook(hook) => {
                if cwd.is_some() {
                    anyhow::bail!("--dir is not valid with the hook subcommand");
                }
                hook.run()
            }
            Command::View {
                dir,
                format,
                full,
                before,
                after,
                args,
            } => {
                let cwd = dir.or(cwd);
                let entries = if args.is_empty() {
                    match crate::state::EditorState::current(cwd.as_deref(), &[])? {
                        Some(state) => state.files,
                        None => return Ok(()),
                    }
                } else if args.len() == 1 && args[0] == "-" {
                    let mut input = String::new();
                    std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)?;
                    crate::view::parse_compact(&input)?
                } else {
                    crate::view::parse_compact(&args.join(" "))?
                };
                let extent = if full {
                    crate::view::Extent::Full
                } else if before.is_some() || after.is_some() {
                    crate::view::Extent::Lines {
                        before: before.unwrap_or(0),
                        after: after.unwrap_or(0),
                    }
                } else {
                    crate::view::Extent::Exact
                };
                match format {
                    Format::Human => {
                        print!("{}", crate::view::render(&entries, cwd.as_deref(), extent)?);
                    }
                    Format::Json => {
                        let payload = crate::view::render_json(&entries, cwd.as_deref(), extent)?;
                        let wrapped = crate::json::Timestamped::now(payload);
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&wrapped).unwrap_or_default()
                        );
                    }
                }
                Ok(())
            }
        }
    }
}
