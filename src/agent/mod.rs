mod claude;
// <-- When adding an agent, add a module for it here

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::cli::Agent;

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
pub fn install(agent: Agent, project: Option<PathBuf>, dev: bool) -> anyhow::Result<()> {
    let bin_cmd = resolve_bin_cmd(dev)?;
    match agent {
        Agent::Claude => claude::install(&bin_cmd, project),
        // <-- Install hooks for future agents go here
    }
}

/// Remove hooks from the agent's settings file.
pub fn uninstall(agent: Agent, project: Option<PathBuf>) -> anyhow::Result<()> {
    match agent {
        Agent::Claude => claude::uninstall(project),
        // <-- Uninstall hooks for future agents go here
    }
}
