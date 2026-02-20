use super::*;

/// Remove hooks from Claude Code settings.
pub fn uninstall(project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let settings_path = settings_path(project.as_deref().map(|p| p.as_std_path()))?;

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
    let hook_keys = [
        HOOK_KEY_SESSION_START,
        HOOK_KEY_USER_PROMPT_SUBMIT,
        HOOK_KEY_STOP,
        HOOK_KEY_PRE_TOOL_USE,
        HOOK_KEY_POST_TOOL_USE,
    ];
    for key in &hook_keys {
        if let Some(arr) = hooks.get_mut(*key).and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|entry| !is_our_hook(entry));
            if arr.len() < before {
                removed = true;
            }
        }
    }

    // Remove attend permissions
    if let Some(perms) = settings
        .get_mut("permissions")
        .and_then(|p| p.as_object_mut())
        && let Some(allow) = perms.get_mut("allow").and_then(|a| a.as_array_mut())
    {
        let before = allow.len();
        allow.retain(|v| v.as_str().map(|s| !s.contains("attend")).unwrap_or(true));
        if allow.len() < before {
            removed = true;
        }
    }

    if removed {
        let mut output =
            serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
        output.push('\n');
        crate::util::atomic_write_str(&settings_path, &output)
            .map_err(|e| anyhow::anyhow!("cannot write settings file: {e}"))?;
        println!("Removed hooks from {}", settings_path.display());
    } else {
        println!("No attend hooks found in {}", settings_path.display());
    }

    Ok(())
}
