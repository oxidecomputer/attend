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
    #[arg(long, short, global = true)]
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

/// Supported agent targets for hook install/uninstall.
#[derive(Clone, ValueEnum)]
pub enum Agent {
    /// Claude Code.
    Claude,
    // <-- Future agents go here
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
        #[arg(long, short)]
        agent: Agent,

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
        #[arg(long, short)]
        agent: Agent,

        /// Remove from a project-local settings file instead of global.
        #[arg(long, short)]
        project: Option<PathBuf>,
    },
}

/// Agent-specific hook runners.
#[derive(Subcommand)]
pub enum RunHook {
    /// Claude Code hooks.
    #[command(subcommand)]
    Claude(ClaudeHook),
    // <-- Future agent hook runners go here
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
        match self.command {
            Some(command) => command.run(self.dir)?,
            None => {
                if let Some(state) = crate::state::EditorState::current(self.dir.as_deref(), None)?
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
            Hook::Run(RunHook::Claude(ClaudeHook::UserPrompt)) => crate::hook::run(cwd),
            Hook::Run(RunHook::Claude(ClaudeHook::SessionStart)) => crate::hook::session_start(),
            // <-- Future agent hook dispatch goes here
            Hook::Install {
                agent,
                project,
                dev,
            } => crate::agent::install(agent, project, dev),
            Hook::Uninstall { agent, project } => crate::agent::uninstall(agent, project),
        }
    }
}
