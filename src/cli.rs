use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Top-level CLI definition.
#[derive(Parser)]
#[command(
    name = "attend",
    about = "Read editor state for AI coding agents.",
    version,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Filter to files under this directory and show relative paths.
    #[arg(long, short)]
    pub dir: Option<PathBuf>,

    /// Output format.
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

/// Value parser that validates agent names against registered backends.
fn agent_value_parser() -> clap::builder::PossibleValuesParser {
    clap::builder::PossibleValuesParser::new(crate::agent::backends().iter().map(|a| a.name()))
}

/// Top-level subcommands.
#[derive(Subcommand)]
pub enum Command {
    /// Hook mode for agent integration.
    #[command(subcommand)]
    Hook(Hook),
}

/// Hook subcommands: run hooks, or manage hook installation.
#[derive(Subcommand)]
pub enum Hook {
    /// Run a hook.
    #[command(subcommand)]
    Run(RunHook),
    /// Install hooks into agent settings.
    Install {
        /// Agent to install for.
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,

        /// Install to a project-local settings file instead of global.
        #[arg(long, short)]
        project: Option<PathBuf>,

        /// Use absolute path to current binary instead of $PATH lookup.
        #[arg(long)]
        dev: bool,
    },
    /// Remove hooks from agent settings.
    Uninstall {
        /// Agent to uninstall for.
        #[arg(long, short, value_parser = agent_value_parser())]
        agent: String,

        /// Remove from a project-local settings file instead of global.
        #[arg(long, short)]
        project: Option<PathBuf>,
    },
}

/// Parsed `hook run <agent> <event>` arguments.
pub struct RunHook {
    pub agent: &'static dyn crate::agent::Agent,
    pub event: crate::agent::HookEvent,
}

impl clap::FromArgMatches for RunHook {
    fn from_arg_matches(matches: &clap::ArgMatches) -> Result<Self, clap::Error> {
        let (agent_name, sub) = matches.subcommand().ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::MissingSubcommand,
                "expected agent name\n",
            )
        })?;
        let agent = crate::agent::backend_by_name(agent_name).ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidSubcommand,
                format!("unknown agent: {agent_name}\n"),
            )
        })?;
        let (event_name, _) = sub.subcommand().ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::MissingSubcommand,
                "expected hook event\n",
            )
        })?;
        let event = crate::agent::HookEvent::from_cli_name(event_name).ok_or_else(|| {
            clap::Error::raw(
                clap::error::ErrorKind::InvalidSubcommand,
                format!("unknown hook event: {event_name}\n"),
            )
        })?;
        Ok(RunHook { agent, event })
    }

    fn update_from_arg_matches(&mut self, matches: &clap::ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
}

impl clap::Subcommand for RunHook {
    fn augment_subcommands(cmd: clap::Command) -> clap::Command {
        let mut cmd = cmd;
        for agent in crate::agent::backends() {
            cmd = cmd.subcommand(crate::agent::clap_command(*agent));
        }
        cmd.subcommand_required(true)
    }

    fn augment_subcommands_for_update(cmd: clap::Command) -> clap::Command {
        Self::augment_subcommands(cmd)
    }

    fn has_subcommand(name: &str) -> bool {
        crate::agent::backend_by_name(name).is_some()
    }
}

impl Cli {
    pub fn run(self) -> anyhow::Result<()> {
        match self.command {
            Some(command) => command.run(self.dir)?,
            None => {
                if let Some(state) =
                    crate::state::EditorState::current(self.dir.as_deref())?
                {
                    match self.format {
                        Format::Human => println!("{state}"),
                        Format::Json => println!(
                            "{}",
                            serde_json::to_string_pretty(&state).unwrap_or_default()
                        ),
                    }
                }
            }
        }

        Ok(())
    }
}

impl Command {
    pub fn run(self, cwd: Option<PathBuf>) -> anyhow::Result<()> {
        match self {
            Command::Hook(hook) => hook.run(cwd),
        }
    }
}

impl Hook {
    pub fn run(self, cwd: Option<PathBuf>) -> anyhow::Result<()> {
        match self {
            Hook::Run(run_hook) => run_hook.agent.run_hook(run_hook.event, cwd),
            Hook::Install {
                agent,
                project,
                dev,
            } => crate::agent::install(&agent, project, dev),
            Hook::Uninstall { agent, project } => crate::agent::uninstall(&agent, project),
        }
    }
}
