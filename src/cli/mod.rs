mod hook;
mod narrate;

pub use hook::HookEvent;
pub use narrate::NarrateCommand;

use camino::Utf8PathBuf;
use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

/// Top-level CLI definition.
#[derive(Parser)]
#[command(
    name = "attend",
    about = "Pair program with your coding agent: editor context and voice narration, delivered seamlessly.",
    version
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

/// Output format.
#[derive(Clone, ValueEnum)]
pub enum Format {
    /// Human-readable text.
    Human,
    /// Pretty-printed JSON.
    Json,
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Record and transcribe voice narration for your agent.
    #[command(display_order = 2, subcommand)]
    Narrate(NarrateCommand),
    /// Show editor state (open files, cursors, selections).
    #[command(display_order = 3)]
    Glance {
        /// Resolve paths relative to this directory and show relative paths.
        #[arg(long, short)]
        dir: Option<Utf8PathBuf>,

        /// Output format.
        #[arg(long, short, default_value = "human")]
        format: Format,

        /// Continuously watch editor state.
        #[arg(long, short)]
        watch: bool,

        /// Override polling / debounce interval in seconds.
        #[arg(long, short = 'i')]
        interval: Option<f64>,
    },
    /// Show file content at cursor and selection positions.
    #[command(display_order = 4)]
    Look {
        /// Resolve paths relative to this directory and show relative paths.
        #[arg(long, short)]
        dir: Option<Utf8PathBuf>,

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

        /// Continuously watch editor state in view mode.
        #[arg(long, short)]
        watch: bool,

        /// Override polling / debounce interval in seconds.
        #[arg(long, short = 'i')]
        interval: Option<f64>,

        /// File paths and positions in compact format (same as default output).
        /// E.g.: src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Run as a background daemon, keeping the state cache warm.
    #[command(display_order = 5)]
    Meditate {
        /// Override polling / debounce interval in seconds.
        #[arg(long, short = 'i')]
        interval: Option<f64>,
    },
    /// Set up agent hooks and editor keybindings.
    #[command(display_order = 6)]
    Install {
        /// Agent to install hooks for (repeatable).
        #[arg(long, short, value_parser = hook::agent_value_parser())]
        agent: Vec<String>,

        /// Editor to install narration keybindings for (repeatable).
        #[arg(long, short, value_parser = hook::editor_value_parser())]
        editor: Vec<String>,

        /// Install to a project-local settings file instead of global.
        #[arg(long, short)]
        project: Option<Utf8PathBuf>,

        /// Use absolute path to current binary instead of $PATH lookup.
        #[arg(long)]
        dev: bool,
    },

    /// Remove agent hooks and editor keybindings.
    #[command(display_order = 7)]
    Uninstall {
        /// Agent to uninstall hooks for (repeatable).
        #[arg(long, short, value_parser = hook::agent_value_parser())]
        agent: Vec<String>,

        /// Editor to uninstall narration keybindings for (repeatable).
        #[arg(long, value_parser = hook::editor_value_parser())]
        editor: Vec<String>,

        /// Remove from a project-local settings file instead of global.
        #[arg(long, short)]
        project: Option<Utf8PathBuf>,
    },
    /// Generate shell completions and print to stdout.
    #[command(display_order = 8)]
    Completions {
        /// Shell to generate completions for.
        shell: clap_complete::Shell,
    },
    /// Receive narration from a recording session.
    #[command(display_order = 9)]
    Listen {
        /// Check once and exit instead of waiting.
        #[arg(long)]
        check: bool,

        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
    },
    /// Respond to agent lifecycle events (used by installed hooks).
    #[command(display_order = 10, subcommand)]
    Hook(HookEvent),
}

impl Cli {
    /// Run the CLI: dispatch to subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        self.command.run()
    }
}

impl Command {
    /// Execute a subcommand.
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            Command::Completions { shell } => {
                clap_complete::generate(
                    shell,
                    &mut Cli::command(),
                    "attend",
                    &mut std::io::stdout(),
                );
                Ok(())
            }
            Command::Glance {
                dir,
                format,
                watch,
                interval,
            } => {
                if watch {
                    crate::watch::run(
                        crate::watch::WatchMode::Compact,
                        dir.as_deref(),
                        interval,
                        &format,
                        false,
                        None,
                        None,
                    )
                } else {
                    if let Some(state) = crate::state::EditorState::current(dir.as_deref(), &[])? {
                        match format {
                            Format::Human => println!("{state}"),
                            Format::Json => {
                                let payload = crate::json::CompactPayload::from_state(&state);
                                let wrapped = crate::util::Timestamped::now(payload);
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
            Command::Look {
                dir,
                format,
                full,
                before,
                after,
                watch,
                interval,
                args,
            } => {
                if watch {
                    crate::watch::run(
                        crate::watch::WatchMode::View,
                        dir.as_deref(),
                        interval,
                        &format,
                        full,
                        before,
                        after,
                    )
                } else {
                    let entries = if args.is_empty() {
                        match crate::state::EditorState::current(dir.as_deref(), &[])? {
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
                            print!("{}", crate::view::render(&entries, dir.as_deref(), extent)?);
                        }
                        Format::Json => {
                            let payload =
                                crate::view::render_json(&entries, dir.as_deref(), extent)?;
                            let wrapped = crate::util::Timestamped::now(payload);
                            println!(
                                "{}",
                                serde_json::to_string_pretty(&wrapped)
                                    .expect("serialization of known type")
                            );
                        }
                    }
                    Ok(())
                }
            }
            Command::Meditate { interval } => crate::watch::run(
                crate::watch::WatchMode::Silent,
                None,
                interval,
                &Format::Human,
                false,
                None,
                None,
            ),
            Command::Listen { check, session } => {
                // check → one-shot (old `receive` without --wait)
                // default → wait (old `receive --wait`)
                crate::narrate::receive::run(!check, session)
            }
            Command::Narrate(cmd) => cmd.run(),
            Command::Hook(event) => event.run(),
            Command::Install {
                agent,
                editor,
                project,
                dev,
            } => {
                if agent.is_empty() && editor.is_empty() {
                    anyhow::bail!(
                        "specify at least one --agent or --editor.\n  Available agents: {}\n  Available editors: {}",
                        crate::agent::AGENTS
                            .iter()
                            .map(|a| a.name())
                            .collect::<Vec<_>>()
                            .join(", "),
                        crate::editor::EDITORS
                            .iter()
                            .map(|e| e.name())
                            .collect::<Vec<_>>()
                            .join(", "),
                    );
                }
                let bin_cmd = crate::agent::resolve_bin_cmd(dev)?;
                for name in &agent {
                    crate::agent::install(name, project.clone(), dev)?;
                }
                for name in &editor {
                    let ed = crate::editor::editor_by_name(name)
                        .ok_or_else(|| anyhow::anyhow!("unknown editor: {name}"))?;
                    ed.install_narration(&bin_cmd)?;
                }
                crate::state::save_install_meta(&crate::state::InstallMeta {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    agents: agent,
                    editors: editor,
                    dev,
                });
                Ok(())
            }
            Command::Uninstall {
                agent,
                editor,
                project,
            } => {
                let uninstall_all = agent.is_empty() && editor.is_empty();
                let agents: Vec<String> = if uninstall_all {
                    crate::agent::AGENTS
                        .iter()
                        .map(|a| a.name().to_string())
                        .collect()
                } else {
                    agent
                };
                let editors: Vec<String> = if uninstall_all {
                    crate::editor::EDITORS
                        .iter()
                        .map(|e| e.name().to_string())
                        .collect()
                } else {
                    editor
                };
                for name in &agents {
                    crate::agent::uninstall(name, project.clone())?;
                }
                for name in &editors {
                    let ed = crate::editor::editor_by_name(name)
                        .ok_or_else(|| anyhow::anyhow!("unknown editor: {name}"))?;
                    ed.uninstall_narration()?;
                }
                Ok(())
            }
        }
    }
}
