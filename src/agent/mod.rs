mod claude;
// <-- Add new agent modules here

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

/// Hook events that agents handle.
#[derive(Clone, Copy)]
pub enum HookEvent {
    SessionStart,
    UserPrompt,
}

impl HookEvent {
    pub const ALL: &[HookEvent] = &[HookEvent::SessionStart, HookEvent::UserPrompt];

    pub fn cli_name(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "session-start",
            HookEvent::UserPrompt => "user-prompt",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "Clear cache and emit instructions for a new session",
            HookEvent::UserPrompt => "Emit editor context for a user prompt",
        }
    }

    pub fn from_cli_name(name: &str) -> Option<HookEvent> {
        match name {
            "session-start" => Some(HookEvent::SessionStart),
            "user-prompt" => Some(HookEvent::UserPrompt),
            _ => None,
        }
    }
}

/// A backend that can install/uninstall hooks and run hook events for an agent.
pub trait Agent: Sync {
    /// CLI name (e.g., "claude").
    fn name(&self) -> &'static str;
    /// Short description for `--help`.
    fn about(&self) -> &'static str;
    /// Run a hook event.
    fn run_hook(&self, event: HookEvent, cwd: Option<PathBuf>) -> anyhow::Result<()>;
    /// Install hooks into agent settings.
    fn install(&self, bin_cmd: &str, project: Option<PathBuf>) -> anyhow::Result<()>;
    /// Remove hooks from agent settings.
    fn uninstall(&self, project: Option<PathBuf>) -> anyhow::Result<()>;
}

/// Build the clap subcommand for an agent (agent name + HookEvent sub-subcommands).
pub fn clap_command(agent: &dyn Agent) -> clap::Command {
    let mut cmd = clap::Command::new(agent.name()).about(agent.about());
    for event in HookEvent::ALL {
        cmd = cmd.subcommand(clap::Command::new(event.cli_name()).about(event.about()));
    }
    cmd.subcommand_required(true)
}

/// All registered agent backends.
pub fn backends() -> &'static [&'static dyn Agent] {
    &[
        &claude::Claude,
        // <-- Add new agents here
    ]
}

/// Look up an agent by CLI name.
pub fn backend_by_name(name: &str) -> Option<&'static dyn Agent> {
    backends().iter().find(|a| a.name() == name).copied()
}

/// Determine the binary command string for hook installation.
fn resolve_bin_cmd(dev: bool) -> anyhow::Result<String> {
    let bin_name = std::env::args()
        .next()
        .map(|a| {
            Path::new(&a)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "attend".to_string());

    if dev {
        Ok(std::env::current_exe()
            .context("cannot determine current exe path")?
            .to_string_lossy()
            .to_string())
    } else {
        match which::which(&bin_name) {
            Ok(_) => Ok(bin_name),
            Err(_) => {
                bail!(
                    "'{bin_name}' not found on $PATH. \
                     Use --dev to use absolute path instead."
                );
            }
        }
    }
}

/// Install hooks into the agent's settings file.
pub fn install(agent_name: &str, project: Option<PathBuf>, dev: bool) -> anyhow::Result<()> {
    let agent = backend_by_name(agent_name)
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {agent_name}"))?;
    let bin_cmd = resolve_bin_cmd(dev)?;
    agent.install(&bin_cmd, project)
}

/// Remove hooks from the agent's settings file.
pub fn uninstall(agent_name: &str, project: Option<PathBuf>) -> anyhow::Result<()> {
    let agent = backend_by_name(agent_name)
        .ok_or_else(|| anyhow::anyhow!("unknown agent: {agent_name}"))?;
    agent.uninstall(project)
}
