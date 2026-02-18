use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;

/// Check whether a hook entry's commands reference `attend` or the given binary command.
fn entry_has_attend_cmd(entry: &serde_json::Value, bin_cmd: Option<&str>) -> bool {
    entry
        .get("hooks")
        .and_then(|h| h.as_array())
        .into_iter()
        .flatten()
        .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
        .any(|cmd| cmd.contains("attend") || bin_cmd.is_some_and(|b| cmd.contains(b)))
}

/// Resolve the Claude Code settings file path (global or project-local).
fn settings_path(project: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(proj) = project {
        Ok(proj.join(".claude").join("settings.json"))
    } else {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".claude").join("settings.json"))
    }
}

/// Install hooks into Claude Code settings.
pub fn install(bin_cmd: &str, project: Option<PathBuf>) -> anyhow::Result<()> {
    let settings_path = settings_path(project.as_deref())?;

    // Read existing settings or start fresh
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).context("cannot read settings file")?;
        serde_json::from_str(&content).context("settings file is not valid JSON")?
    } else {
        serde_json::json!({})
    };

    let obj = settings
        .as_object_mut()
        .context("settings is not an object")?;

    // Build hook commands
    let session_start_cmd = format!("{bin_cmd} hook run claude session-start");
    let prompt_cmd = format!("{bin_cmd} hook run claude user-prompt");

    // Build the hooks structure
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().context("hooks is not an object")?;

    // SessionStart
    let session_start_hook = serde_json::json!({
        "matcher": "startup|clear|compact",
        "hooks": [
            {
                "type": "command",
                "command": session_start_cmd
            }
        ]
    });

    let ss_arr = hooks_obj
        .entry("SessionStart")
        .or_insert_with(|| serde_json::json!([]));
    let ss_vec = ss_arr
        .as_array_mut()
        .context("SessionStart is not an array")?;

    // Remove existing attend entries (idempotent)
    let before = ss_vec.len();
    ss_vec.retain(|entry| !entry_has_attend_cmd(entry, Some(bin_cmd)));
    let mut existing = before > ss_vec.len();
    ss_vec.push(session_start_hook);

    // UserPromptSubmit
    let prompt_hook = serde_json::json!({
        "hooks": [
            {
                "type": "command",
                "command": prompt_cmd,
                "timeout": 5
            }
        ]
    });

    let ups_arr = hooks_obj
        .entry("UserPromptSubmit")
        .or_insert_with(|| serde_json::json!([]));
    let ups_vec = ups_arr
        .as_array_mut()
        .context("UserPromptSubmit is not an array")?;

    let before = ups_vec.len();
    ups_vec.retain(|entry| !entry_has_attend_cmd(entry, Some(bin_cmd)));
    existing = existing || before > ups_vec.len();
    ups_vec.push(prompt_hook);

    // Write back
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).context("cannot create settings directory")?;
    }
    let mut output =
        serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
    output.push('\n');
    fs::write(&settings_path, output).context("cannot write settings file")?;

    if existing {
        println!("Updated existing hooks in {}", settings_path.display());
    } else {
        println!("Installed hooks to {}", settings_path.display());
    }
    Ok(())
}

/// Remove hooks from Claude Code settings.
pub fn uninstall(project: Option<PathBuf>) -> anyhow::Result<()> {
    let settings_path = settings_path(project.as_deref())?;

    if !settings_path.exists() {
        println!("No settings file found at {}", settings_path.display());
        return Ok(());
    }

    let content = fs::read_to_string(&settings_path).context("cannot read settings file")?;
    let mut settings: serde_json::Value =
        serde_json::from_str(&content).context("settings file is not valid JSON")?;

    let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        println!("No hooks found in {}", settings_path.display());
        return Ok(());
    };

    let mut removed = false;
    for key in &["SessionStart", "UserPromptSubmit"] {
        if let Some(arr) = hooks.get_mut(*key).and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|entry| !entry_has_attend_cmd(entry, None));
            if arr.len() < before {
                removed = true;
            }
        }
    }

    if removed {
        let mut output =
            serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
        output.push('\n');
        fs::write(&settings_path, output).context("cannot write settings file")?;
        println!("Removed hooks from {}", settings_path.display());
    } else {
        println!("No attend hooks found in {}", settings_path.display());
    }

    Ok(())
}
