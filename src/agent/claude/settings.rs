use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use camino::Utf8PathBuf;

/// Claude Code hook configuration keys.
const HOOK_KEY_SESSION_START: &str = "SessionStart";
const HOOK_KEY_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const HOOK_KEY_STOP: &str = "Stop";
const HOOK_KEY_PRE_TOOL_USE: &str = "PreToolUse";
const HOOK_KEY_POST_TOOL_USE: &str = "PostToolUse";
const HOOK_KEY_SESSION_END: &str = "SessionEnd";

/// Marker key added to every hook entry we install.
///
/// Used for precise identification during idempotent re-install and
/// uninstall — avoids false positives from substring matching on
/// command strings.
const HOOK_MARKER_KEY: &str = "_installed_by";
const HOOK_MARKER_VALUE: &str = "attend";

/// Check whether a hook entry was installed by attend.
///
/// Primary check: the `_installed_by` marker added by current versions.
/// Fallback: match legacy entries (pre-marker) by command pattern to
/// prevent unbounded accumulation on reinstall/upgrade.
fn is_our_hook(entry: &serde_json::Value) -> bool {
    if entry.get(HOOK_MARKER_KEY).and_then(|v| v.as_str()) == Some(HOOK_MARKER_VALUE) {
        return true;
    }
    // Legacy fallback: check if any hook command starts with "attend hook".
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(|cmd| {
                        let trimmed = cmd.trim();
                        trimmed == "attend" || trimmed.starts_with("attend ")
                    })
            })
        })
}

/// Resolve the Claude Code settings file path.
///
/// Global installs use `~/.claude/settings.json`. Project installs use
/// `settings.local.json` so as not to impose the tool on other contributors.
fn settings_path(project: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(proj) = project {
        Ok(proj.join(".claude").join("settings.local.json"))
    } else {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".claude").join("settings.json"))
    }
}

pub mod install;
pub mod uninstall;
pub use {install::install, uninstall::uninstall};

#[cfg(test)]
mod tests;
