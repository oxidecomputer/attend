//! Health checks for Zed narration integration.

use super::jsonc::read_jsonc_array;
use super::keybindings::is_narration_keybinding;
use super::{NARRATION_TASK_LABELS, zed_config_dir};

/// Check whether a command exists — either as an absolute path or on PATH.
fn command_exists(cmd: &str) -> bool {
    let path = std::path::Path::new(cmd);
    if path.is_absolute() {
        path.exists()
    } else {
        which::which(cmd).is_ok()
    }
}

/// Check health of installed Zed narration integration.
pub(super) fn check_narration_health() -> anyhow::Result<Vec<String>> {
    let reinstall = "run `attend install --editor zed`";
    let mut warnings = Vec::new();

    // Check tasks
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let tasks = read_jsonc_array(&tasks_path);

    if tasks.is_empty() && !tasks_path.exists() {
        warnings.push(format!("tasks.json not found: {reinstall}"));
    } else {
        for label in NARRATION_TASK_LABELS {
            let task = tasks
                .iter()
                .find(|t| t.get("label").and_then(|l| l.as_str()) == Some(label));
            match task {
                None => warnings.push(format!("{label} task not found: {reinstall}")),
                Some(t) => {
                    if let Some(cmd) = t.get("command").and_then(|c| c.as_str())
                        && !command_exists(cmd)
                    {
                        warnings.push(format!(
                            "task command path does not exist: {cmd}: reinstall with {reinstall}"
                        ));
                    }
                }
            }
        }
    }

    // Check keybindings
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let keymap = read_jsonc_array(&keymap_path);

    if keymap.is_empty() && !keymap_path.exists() {
        warnings.push(format!("keymap.json not found: {reinstall}"));
    } else if !keymap.iter().any(is_narration_keybinding) {
        warnings.push(format!("narration keybinding not found: {reinstall}"));
    }

    Ok(warnings)
}
