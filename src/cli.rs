use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

/// Top-level CLI definition.
#[derive(Parser)]
#[command(name = "zed-context", about = "Read Zed editor state.", version, args_conflicts_with_subcommands = true)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,

    /// Filter to files under this directory and show relative paths.
    #[arg(long, global = true)]
    pub cwd: Option<PathBuf>,

    /// Output format.
    #[arg(long, default_value = "human")]
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

/// Supported agent targets for hook install/uninstall.
#[derive(Clone, ValueEnum)]
pub enum Agent {
    /// Claude Code.
    Claude,
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
        #[arg(long)]
        agent: Agent,

        /// Install to a project-local settings file instead of global.
        #[arg(long)]
        project: Option<PathBuf>,

        /// Use absolute path to current binary instead of $PATH lookup.
        #[arg(long)]
        dev: bool,
    },
    /// Remove hooks from agent settings.
    Uninstall {
        /// Agent to uninstall for.
        #[arg(long)]
        agent: Agent,

        /// Remove from a project-local settings file instead of global.
        #[arg(long)]
        project: Option<PathBuf>,
    },
}

/// Agent-specific hook runners.
#[derive(Subcommand)]
pub enum RunHook {
    /// Claude Code hooks.
    #[command(subcommand)]
    Claude(ClaudeHook),
}

/// Individual Claude Code hook events.
#[derive(Subcommand)]
pub enum ClaudeHook {
    /// Emit editor context for a user prompt.
    UserPrompt,
    /// Clear cache and emit instructions for a new session.
    SessionStart,
}

impl Cli {
    pub fn run(self) -> anyhow::Result<()> {
        use crate::model;

        match self.command {
            Some(command) => command.run(self.cwd)?,
            None => {
                if let Some(state) = model::get_editor_state(self.cwd.as_deref())? {
                    match self.format {
                        Format::Human => println!("{state}"),
                        Format::Json => println!("{}", serde_json::to_string_pretty(&state).unwrap_or_default()),
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
        use crate::hook;

        match self {
            Hook::Run(RunHook::Claude(ClaudeHook::UserPrompt)) => hook::run(cwd),
            Hook::Run(RunHook::Claude(ClaudeHook::SessionStart)) => hook::session_start(),
            Hook::Install { agent, project, dev } => hook::install(agent, project, dev),
            Hook::Uninstall { agent, project } => hook::uninstall(agent, project),
        }
    }
}
