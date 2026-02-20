//! Zed keybinding install/uninstall for narration.

use super::jsonc::{read_jsonc_array, write_json_array};
use super::{NARRATION_KEYS, zed_config_dir};

/// Install a Zed keybinding for a narration task.
///
/// Skips installation if the task is already bound to any key (user may have
/// reassigned it) or if the default key is already bound to something else.
pub(super) fn install_keybinding(key: &str, task_name: &str) -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let mut keymap = read_jsonc_array(&keymap_path);

    let task_already_bound = keymap.iter().any(|e| {
        e.get("bindings")
            .and_then(|b| b.as_object())
            .is_some_and(|b| {
                b.values().any(|v| {
                    v.as_array().is_some_and(|a| {
                        a.first().and_then(|s| s.as_str()) == Some("task::Spawn")
                            && a.get(1)
                                .and_then(|o| o.get("task_name"))
                                .and_then(|n| n.as_str())
                                == Some(task_name)
                    })
                })
            })
    });
    if task_already_bound {
        return Ok(());
    }

    let key_already_bound = keymap.iter().any(|e| {
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
    write_json_array(&keymap_path, &keymap)
}

/// Remove the Zed keybindings for narration.
pub(super) fn uninstall_keybinding() -> anyhow::Result<()> {
    let keymap_path = zed_config_dir()?.join("keymap.json");
    let mut keymap = read_jsonc_array(&keymap_path);

    let before = keymap.len();
    keymap.retain(|entry| !is_narration_keybinding(entry));

    if keymap.len() < before {
        write_json_array(&keymap_path, &keymap)?;
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
