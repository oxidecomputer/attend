mod browser_bridge;
mod completions;
mod glance;
mod hook;
mod install;
mod listen;
mod look;
mod meditate;
mod narrate;
mod shell_hook;

pub use hook::HookEvent;
pub use narrate::NarrateCommand;

use clap::{Parser, Subcommand, ValueEnum};

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
    Glance(glance::GlanceArgs),
    /// Show file content at cursor and selection positions.
    #[command(display_order = 4)]
    Look(look::LookArgs),
    /// Run as a background daemon, keeping the state cache warm.
    #[command(display_order = 5)]
    Meditate(meditate::MeditateArgs),
    /// Set up agent hooks, editor keybindings, browser, and shell integrations.
    #[command(display_order = 6)]
    Install(install::InstallArgs),
    /// Remove agent hooks, editor keybindings, browser, and shell integrations.
    #[command(display_order = 7)]
    Uninstall(install::UninstallArgs),
    /// Generate shell completions and print to stdout.
    #[command(display_order = 8)]
    Completions(completions::CompletionsArgs),
    /// Receive narration from a recording session.
    #[command(display_order = 9)]
    Listen(listen::ListenArgs),
    /// Respond to agent lifecycle events (used by installed hooks).
    #[command(display_order = 10, subcommand)]
    Hook(HookEvent),
    /// Native messaging host for browser extensions (internal).
    #[command(display_order = 11, hide = true)]
    BrowserBridge,
    /// Stage shell command events for narration (internal, called by shell hooks).
    #[command(display_order = 12, hide = true, subcommand)]
    ShellHook(ShellHookCommand),
}

/// Shell hook subcommands.
#[derive(Subcommand)]
pub enum ShellHookCommand {
    /// Stage a preexec event (command starting).
    Preexec {
        /// Shell name (e.g. "fish", "zsh").
        #[arg(long)]
        shell: String,
        /// The command as typed by the user.
        #[arg(long)]
        command: String,
    },
    /// Stage a postexec event (command completed).
    Postexec {
        /// Shell name (e.g. "fish", "zsh").
        #[arg(long)]
        shell: String,
        /// The command as typed by the user.
        #[arg(long)]
        command: String,
        /// Exit status of the command.
        #[arg(long)]
        exit_status: i32,
        /// Wall-clock duration in seconds.
        #[arg(long)]
        duration: f64,
    },
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
            Command::Narrate(cmd) => cmd.run(),
            Command::Glance(args) => args.run(),
            Command::Look(args) => args.run(),
            Command::Meditate(args) => args.run(),
            Command::Completions(args) => args.run(),
            Command::Listen(args) => args.run(),
            Command::Hook(event) => event.run(),
            Command::BrowserBridge => browser_bridge::run(),
            Command::ShellHook(cmd) => match cmd {
                ShellHookCommand::Preexec { shell, command } => shell_hook::preexec(shell, command),
                ShellHookCommand::Postexec {
                    shell,
                    command,
                    exit_status,
                    duration,
                } => shell_hook::postexec(shell, command, exit_status, duration),
            },
            Command::Install(args) => args.run(),
            Command::Uninstall(args) => args.run(),
        }
    }
}
