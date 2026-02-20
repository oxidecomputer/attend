//! Zed task definition install/uninstall for narration.

use super::jsonc::{read_jsonc_array, write_json_array};
use super::{NARRATION_TASK_LABELS, zed_config_dir};

/// Legacy task labels from previous versions.
const LEGACY_TASK_LABELS: &[&str] = &[
    "Toggle Dictation",
    "Flush Dictation",
    "Toggle Narration",
    "Flush Narration",
    "attend: flush narration",
];

/// Install a Zed task definition for narration.
pub(super) fn install_task(bin_cmd: &str, label: &str, args: &[&str]) -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let mut tasks = read_jsonc_array(&tasks_path);

    let task_entry = serde_json::json!({
        "label": label,
        "command": bin_cmd,
        "args": args,
        "hide": "always",
        "reveal": "never",
        "allow_concurrent_runs": false,
        "use_new_terminal": true
    });

    if tasks.contains(&task_entry) {
        return Ok(());
    }

    // Warn about stale command path before replacing.
    if let Some(cmd) = tasks
        .iter()
        .find(|t| t.get("label").and_then(|l| l.as_str()) == Some(label))
        .and_then(|t| t.get("command").and_then(|c| c.as_str()))
        && !std::path::Path::new(cmd).exists()
    {
        tracing::warn!("Replacing stale command path: {cmd}");
    }

    // Remove both current and legacy labels
    tasks.retain(|t| {
        let l = t.get("label").and_then(|l| l.as_str());
        l != Some(label) && !l.is_some_and(|l| LEGACY_TASK_LABELS.contains(&l))
    });
    tasks.push(task_entry);
    write_json_array(&tasks_path, &tasks)
}

/// Remove the Zed task definitions for narration (current + legacy).
pub(super) fn uninstall_task() -> anyhow::Result<()> {
    let tasks_path = zed_config_dir()?.join("tasks.json");
    let mut tasks = read_jsonc_array(&tasks_path);

    let before = tasks.len();
    tasks.retain(|t| {
        let label = t.get("label").and_then(|l| l.as_str());
        !label
            .is_some_and(|l| NARRATION_TASK_LABELS.contains(&l) || LEGACY_TASK_LABELS.contains(&l))
    });

    if tasks.len() < before {
        write_json_array(&tasks_path, &tasks)?;
    }
    Ok(())
}
