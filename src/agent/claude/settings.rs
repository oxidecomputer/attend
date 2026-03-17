use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use camino::Utf8PathBuf;

/// Shared hook definitions: the single source of truth for which Claude Code
/// hooks attend installs, their subcommands, timeouts, and matchers.
///
/// Consumed by `install.rs` (manual hook injection into settings.json),
/// `uninstall.rs` (removal), and `cargo xtask sync-plugin` (plugin
/// hooks.json generation).
const HOOK_DEFS_JSON: &str = include_str!("hook_defs.json");

/// Parsed hook definition from `hook_defs.json`.
#[derive(serde::Deserialize)]
pub(super) struct HookDef {
    pub event: String,
    pub subcommand: String,
    #[serde(default)]
    pub matcher: Option<String>,
    #[serde(default)]
    pub timeout: Option<u64>,
}

/// Parse hook definitions from the embedded JSON.
pub(super) fn hook_defs() -> Vec<HookDef> {
    serde_json::from_str(HOOK_DEFS_JSON).expect("hook_defs.json is invalid")
}

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
    // Fallback: match by command pattern. Claude Code strips the
    // _installed_by marker on session start, so this is the primary
    // detection path in practice. Matches both bare "attend" and
    // absolute-path "/path/to/attend" (dev installs).
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .is_some_and(|hooks| {
            hooks.iter().any(|h| {
                h.get("command")
                    .and_then(|c| c.as_str())
                    .is_some_and(is_attend_command)
            })
        })
}

/// Check whether a command string invokes `attend`.
///
/// Matches both `"attend hook ..."` (PATH install) and
/// `"/path/to/attend hook ..."` (dev install with absolute path).
fn is_attend_command(cmd: &str) -> bool {
    let first_word = cmd.split_whitespace().next().unwrap_or("");
    first_word == "attend"
        || first_word
            .rsplit('/')
            .next()
            .is_some_and(|basename| basename == "attend")
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
