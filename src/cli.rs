mod browser_bridge;
mod glance;
mod hook;
mod install;
mod look;
mod narrate;
mod shell_hook;

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
    /// Set up agent hooks, editor keybindings, browser, and shell integrations.
    #[command(display_order = 6)]
    Install {
        /// Agent to install hooks for (repeatable).
        #[arg(long, short, value_parser = hook::agent_value_parser())]
        agent: Vec<String>,

        /// Editor to install narration keybindings for (repeatable).
        #[arg(long, short, value_parser = hook::editor_value_parser())]
        editor: Vec<String>,

        /// Browser to install native messaging for (repeatable).
        #[arg(long, short, value_parser = hook::browser_value_parser())]
        browser: Vec<String>,

        /// Shell to install hooks and completions for (repeatable).
        #[arg(long, short, value_parser = hook::shell_value_parser())]
        shell: Vec<String>,

        /// Install to a project-local settings file instead of global.
        #[arg(long, short)]
        project: Option<Utf8PathBuf>,

        /// Use absolute path to current binary instead of $PATH lookup.
        #[arg(long)]
        dev: bool,
    },

    /// Remove agent hooks, editor keybindings, browser, and shell integrations.
    #[command(display_order = 7)]
    Uninstall {
        /// Agent to uninstall hooks for (repeatable).
        #[arg(long, short, value_parser = hook::agent_value_parser())]
        agent: Vec<String>,

        /// Editor to uninstall narration keybindings for (repeatable).
        #[arg(long, value_parser = hook::editor_value_parser())]
        editor: Vec<String>,

        /// Browser to uninstall native messaging for (repeatable).
        #[arg(long, short, value_parser = hook::browser_value_parser())]
        browser: Vec<String>,

        /// Shell to uninstall hooks and completions for (repeatable).
        #[arg(long, short, value_parser = hook::shell_value_parser())]
        shell: Vec<String>,

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
            } => glance::run(dir, format, watch, interval),
            Command::Look {
                dir,
                format,
                full,
                before,
                after,
                watch,
                interval,
                args,
            } => look::run(dir, format, full, before, after, watch, interval, args),
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
            Command::Install {
                agent,
                editor,
                browser,
                shell,
                project,
                dev,
            } => install::install(agent, editor, browser, shell, project, dev),
            Command::Uninstall {
                agent,
                editor,
                browser,
                shell,
                project,
            } => install::uninstall(agent, editor, browser, shell, project),
        }
    }
}
