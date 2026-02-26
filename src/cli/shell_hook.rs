//! CLI handler for `attend shell-hook preexec|postexec`.
//!
//! Called by fish/zsh hooks to stage shell command events during recording.
//! Designed to be fast: checks record lock, resolves session, writes one
//! JSON file, exits. No model loading, no blocking.

use std::fs;

use clap::Subcommand;

use crate::narrate::merge::Event;
use crate::narrate::shell_staging_dir;
use crate::state;
use crate::util;

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

impl ShellHookCommand {
    pub fn run(self) -> anyhow::Result<()> {
        match self {
            ShellHookCommand::Preexec { shell, command } => stage_event(shell, command, None, None),
            ShellHookCommand::Postexec {
                shell,
                command,
                exit_status,
                duration,
            } => stage_event(shell, command, Some(exit_status), Some(duration)),
        }
    }
}

/// Write a `ShellCommand` event to the shell staging directory.
fn stage_event(
    shell: String,
    command: String,
    exit_status: Option<i32>,
    duration_secs: Option<f64>,
) -> anyhow::Result<()> {
    // Only stage events while narration is actively recording.
    if !crate::narrate::record_lock_path().exists() {
        return Ok(());
    }

    // Resolve the session, if any. When no agent session is active the
    // event is staged to the `_local` directory so it is still captured.
    let session_id = state::listening_session();

    // Capture the shell's working directory.
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.to_str().map(String::from))
        .unwrap_or_default();

    let events = vec![Event::ShellCommand {
        // Placeholder: the recording daemon overwrites this with the
        // UTC timestamp parsed from the staging filename.
        timestamp: chrono::Utc::now(),
        shell,
        command,
        cwd,
        exit_status,
        duration_secs,
    }];

    let json = serde_json::to_string(&events)?;
    let ts = util::utc_now().replace(':', "-");
    let dir = shell_staging_dir(session_id.as_ref());
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{ts}.json"));
    util::atomic_write_str(&path, &json)?;

    Ok(())
}
