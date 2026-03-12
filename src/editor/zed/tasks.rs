//! Zed task definition install/uninstall for narration.

use super::jsonc::JsoncArray;
use super::{NARRATION_TASK_LABELS, zed_config_dir};

/// Install a Zed task definition for narration.
pub(super) fn install_task(bin_cmd: &str, label: &str, args: &[&str]) -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let mut tasks = JsoncArray::open(&tasks_path)?;

    let task_entry = serde_json::json!({
        "label": label,
        "command": bin_cmd,
        "args": args,
        "hide": "always",
        "reveal": "never",
        "allow_concurrent_runs": false,
        "use_new_terminal": true
    });

    if tasks.elements().contains(&task_entry) {
        return Ok(());
    }

    // Warn about stale command path before replacing.
    if let Some(cmd) = tasks
        .elements()
        .iter()
        .find(|t| t.get("label").and_then(|l| l.as_str()) == Some(label))
        .and_then(|t| t.get("command").and_then(|c| c.as_str()))
        .map(|s| s.to_string())
        && !std::path::Path::new(&cmd).exists()
    {
        tracing::warn!("Replacing stale command path: {cmd}");
    }

    tasks.retain(|t| t.get("label").and_then(|l| l.as_str()) != Some(label));
    tasks.push(task_entry);
    tasks.save()
}

/// Remove the Zed task definitions for narration.
pub(super) fn uninstall_task() -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let mut tasks = JsoncArray::open(&tasks_path)?;

    tasks.retain(|t| {
        let label = t.get("label").and_then(|l| l.as_str());
        !label.is_some_and(|l| NARRATION_TASK_LABELS.contains(&l))
    });

    if tasks.is_modified() {
        tasks.save()?;
    }
    Ok(())
}
