mod claude;
// <-- Add new agent modules here

use std::path::{Path, PathBuf};

use anyhow::Context;

/// Hook events that agents handle.
#[derive(Clone, Copy)]
pub enum HookEvent {
    /// Fired once at the start of an agent session.
    SessionStart,
    /// Fired before each user prompt is sent.
    UserPrompt,
    /// Fired when the agent session stops.
    Stop,
}

/// A backend that can install/uninstall hooks and run hook events for an agent.
pub trait Agent: Sync {
    /// CLI name (e.g., "claude").
    fn name(&self) -> &'static str;
    /// Full name (e.g. "Claude Code").
    fn full_name(&self) -> &'static str;
    /// Run a hook event.
    fn run_hook(&self, event: HookEvent, cwd: Option<PathBuf>) -> anyhow::Result<()>;
    /// Install hooks into agent settings.
    fn install(&self, bin_cmd: &str, project: Option<PathBuf>) -> anyhow::Result<()>;
    /// Remove hooks from agent settings.
    fn uninstall(&self, project: Option<PathBuf>) -> anyhow::Result<()>;
}

/// All registered agent backends.
pub const AGENTS: &[&'static dyn Agent] = &[
    &claude::Claude,
    // <-- Add new agents here
];

/// Look up an agent by CLI name.
pub fn backend_by_name(name: &str) -> Option<&'static dyn Agent> {
    AGENTS.iter().find(|a| a.name() == name).copied()
}

/// Determine the binary command string for hook installation.
pub(crate) fn resolve_bin_cmd(dev: bool) -> anyhow::Result<String> {
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
            Err(_) => Ok(std::env::current_exe()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or(bin_name)),
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
