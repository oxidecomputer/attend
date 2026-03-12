//! Zed keybinding install/uninstall for narration.

use super::jsonc::JsoncArray;
use super::{NARRATION_KEYS, zed_config_dir};

/// Yield task names from `task::Spawn` bindings in a keymap entry.
///
/// Each Zed keymap entry has `{ "bindings": { "<key>": ["task::Spawn", {"task_name": "..."}] } }`.
/// This drills through that structure and yields the `task_name` for every
/// binding whose action is `task::Spawn`.
fn bound_task_names(entry: &serde_json::Value) -> impl Iterator<Item = &str> {
    entry
        .get("bindings")
        .and_then(|b| b.as_object())
        .into_iter()
        .flat_map(|bindings| bindings.values())
        .filter_map(|v| v.as_array())
        .filter(|a| a.first().and_then(|s| s.as_str()) == Some("task::Spawn"))
        .filter_map(|a| a.get(1)?.get("task_name")?.as_str())
}

/// Install a Zed keybinding for a narration task.
///
/// Skips installation if the task is already bound to any key (user may have
/// reassigned it) or if the default key is already bound to something else.
pub(super) fn install_keybinding(key: &str, task_name: &str) -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let mut keymap = JsoncArray::open(&keymap_path)?;
    let elements = keymap.elements();

    let task_already_bound = elements
        .iter()
        .any(|e| bound_task_names(e).any(|n| n == task_name));
    if task_already_bound {
        return Ok(());
    }

    let key_already_bound = elements.iter().any(|e| {
        e.get("bindings")
            .and_then(|b| b.as_object())
            .is_some_and(|b| b.contains_key(key))
    });
    if key_already_bound {
        return Ok(());
    }

    let mut bindings = serde_json::Map::new();
    bindings.insert(
        key.to_string(),
        serde_json::json!(["task::Spawn", {"task_name": task_name}]),
    );
    keymap.push(serde_json::json!({ "bindings": bindings }));
    keymap.save()
}

/// Remove the Zed keybindings for narration.
pub(super) fn uninstall_keybinding() -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let mut keymap = JsoncArray::open(&keymap_path)?;

    keymap.retain(|entry| !is_narration_keybinding(entry));

    if keymap.is_modified() {
        keymap.save()?;
    }
    Ok(())
}

/// Check whether a keymap entry is solely our narration keybinding.
pub(super) fn is_narration_keybinding(entry: &serde_json::Value) -> bool {
    let Some(bindings) = entry.get("bindings").and_then(|b| b.as_object()) else {
        return false;
    };
    if bindings.len() != 1 {
        return false;
    }
    NARRATION_KEYS.iter().any(|key| {
        bindings
            .get(*key)
            .and_then(|v| v.as_array())
            .is_some_and(|a| a.first().and_then(|s| s.as_str()) == Some("task::Spawn"))
    })
}
