mod db;
mod health;
pub(super) mod jsonc;
mod keybindings;
mod tasks;

use std::path::PathBuf;

use anyhow::Context;

use super::{Editor, QueryResult};

/// Known narration keybindings (current + legacy).
const NARRATION_KEYS: &[&str] = &[
    "cmd-:", // start (current)
    "cmd-;", // toggle (current)
];

/// Narration task labels.
const NARRATION_TASK_LABELS: &[&str] = &["attend: toggle narration", "attend: start narration"];

/// Zed config directory (`~/.config/zed`).
///
/// Zed uses `~/.config/zed` on all platforms, not the platform-native
/// config directory (e.g. `~/Library/Application Support` on macOS).
fn zed_config_dir() -> anyhow::Result<PathBuf> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    Ok(home.join(".config").join("zed"))
}

/// Zed editor backend — queries the Zed SQLite database for open tabs.
pub struct Zed;

impl Editor for Zed {
    fn name(&self) -> &'static str {
        "zed"
    }

    fn query(&self) -> anyhow::Result<Option<QueryResult>> {
        query()
    }

    fn install_narration(&self, bin_cmd: &str) -> anyhow::Result<()> {
        tasks::install_task(bin_cmd, "attend: toggle narration", &["narrate", "toggle"])?;
        tasks::install_task(bin_cmd, "attend: start narration", &["narrate", "start"])?;
        keybindings::install_keybinding("cmd-;", "attend: toggle narration")?;
        keybindings::install_keybinding("cmd-:", "attend: start narration")?;
        println!("Installed Zed narration tasks and keybindings.");
        Ok(())
    }

    fn uninstall_narration(&self) -> anyhow::Result<()> {
        tasks::uninstall_task()?;
        keybindings::uninstall_keybinding()?;
        println!("Removed Zed narration task and keybinding.");
        Ok(())
    }

    fn check_narration(&self) -> anyhow::Result<Vec<String>> {
        health::check_narration_health()
    }
}

fn query() -> anyhow::Result<Option<QueryResult>> {
    let db_path = match db::find_db() {
        Some(p) => p,
        None => return Ok(None),
    };

    let conn = rusqlite::Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .context("failed to open DB")?;

    let editors = db::query_editors(&conn)?;

    Ok(Some(QueryResult { editors }))
}

#[cfg(test)]
mod tests;
