use super::*;

/// Check whether the attend plugin is enabled in the given settings.
fn plugin_enabled(settings: &serde_json::Value) -> bool {
    settings
        .get("enabledPlugins")
        .and_then(|p| p.get("attend@attend"))
        .and_then(|v| v.as_bool())
        == Some(true)
}

/// Install hooks into Claude Code settings.
///
/// If the attend plugin is already enabled, only permissions are written
/// (the plugin provides hooks and skills). Otherwise, the full manual
/// install is performed: hooks, permissions, and skill files.
pub fn install(bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let settings_path = settings_path(project.as_deref().map(|p| p.as_std_path()))?;

    // Read existing settings or start fresh
    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path).context("cannot read settings file")?;
        serde_json::from_str(&content).context("settings file is not valid JSON")?
    } else {
        serde_json::json!({})
    };

    let plugin = plugin_enabled(&settings);

    let obj = settings
        .as_object_mut()
        .context("settings is not an object")?;

    // If the plugin provides hooks and skills, skip manual installation
    // of those. Only write permissions (which plugins cannot set).
    let mut existing = false;
    if !plugin {
        let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
        let hooks_obj = hooks.as_object_mut().context("hooks is not an object")?;

        let defs = hook_defs();

        for def in &defs {
            let cmd = format!("{bin_cmd} hook {} --agent claude", def.subcommand);

            // Build the inner hook entry.
            let mut inner = serde_json::json!({
                "type": "command",
                "command": cmd,
            });
            if let Some(timeout) = def.timeout {
                inner["timeout"] = serde_json::json!(timeout);
            }

            // Build the outer hook group with our marker.
            let mut entry = serde_json::json!({
                HOOK_MARKER_KEY: HOOK_MARKER_VALUE,
                "hooks": [inner],
            });
            if let Some(ref matcher) = def.matcher {
                entry["matcher"] = serde_json::json!(matcher);
            }

            // Insert into the appropriate event array (idempotent).
            let arr = hooks_obj
                .entry(&def.event)
                .or_insert_with(|| serde_json::json!([]));
            let vec = arr
                .as_array_mut()
                .context(format!("{} is not an array", def.event))?;

            let before = vec.len();
            vec.retain(|e| !is_our_hook(e));
            existing = existing || before > vec.len();
            vec.push(entry);
        }
    }

    // Pre-authorize attend commands so Claude doesn't prompt.
    // Always written regardless of plugin status, since plugins
    // cannot set permissions.
    install_permissions(obj, bin_cmd)?;

    // Write back
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).context("cannot create settings directory")?;
    }
    let mut output =
        serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
    output.push('\n');
    crate::util::atomic_write_str(&settings_path, &output)
        .map_err(|e| anyhow::anyhow!("cannot write settings file: {e}"))?;

    if plugin {
        println!(
            "Installed permissions to {} (hooks and skills provided by plugin)",
            settings_path.display()
        );
    } else {
        // SKILL.md for /attend discoverability
        install_skill_file(bin_cmd, project.as_deref().map(|p| p.as_std_path()))?;

        if existing {
            println!("Updated existing hooks in {}", settings_path.display());
        } else {
            println!("Installed hooks to {}", settings_path.display());
        }
    }
    Ok(())
}

/// Write attend permission patterns into settings.
fn install_permissions(
    obj: &mut serde_json::Map<String, serde_json::Value>,
    bin_cmd: &str,
) -> anyhow::Result<()> {
    let permissions = obj
        .entry("permissions")
        .or_insert_with(|| serde_json::json!({}));
    let perms_obj = permissions
        .as_object_mut()
        .context("permissions is not an object")?;
    let allow = perms_obj
        .entry("allow")
        .or_insert_with(|| serde_json::json!([]));
    let allow_vec = allow
        .as_array_mut()
        .context("permissions.allow is not an array")?;

    let look_pattern = format!("Bash({bin_cmd} look:*)");
    let listen_pattern = format!("Bash({bin_cmd} listen:*)");
    // Claude Code uses gitignore-style patterns: `~/` = relative to home
    // directory, which is portable and avoids the single-`/` trap (which
    // means relative to project root, not filesystem root).
    let clipboard_root_abs = crate::narrate::clipboard_staging_root();
    let clipboard_root_tilde = dirs::home_dir()
        .and_then(|home| {
            clipboard_root_abs
                .as_str()
                .strip_prefix(home.to_str()?)
                .map(|rel| format!("~{rel}"))
        })
        .unwrap_or_else(|| clipboard_root_abs.to_string());
    let clipboard_read_pattern = format!("Read({clipboard_root_tilde}/**)");
    // Remove our own entries, then re-add current.
    // Match on the exact patterns we install, not a substring search,
    // to avoid clobbering unrelated permissions.
    // Also remove legacy clipboard patterns (flat `/*`, `/*/*`, `/**`).
    let clipboard_root_str = clipboard_root_abs.to_string();
    allow_vec.retain(|v: &serde_json::Value| {
        v.as_str()
            .map(|s| {
                s != look_pattern
                    && s != listen_pattern
                    && !s.starts_with(&format!("Read({clipboard_root_str}"))
                    && !s.starts_with(&format!("Read({clipboard_root_tilde}"))
            })
            .unwrap_or(true)
    });
    allow_vec.push(serde_json::Value::String(look_pattern));
    allow_vec.push(serde_json::Value::String(listen_pattern));
    allow_vec.push(serde_json::Value::String(clipboard_read_pattern));

    Ok(())
}

/// Install SKILL.md files for `/attend` and `/unattend` discoverability.
fn install_skill_file(bin_cmd: &str, project: Option<&Path>) -> anyhow::Result<()> {
    let base = if let Some(proj) = project {
        proj.to_path_buf()
    } else {
        dirs::home_dir().context("cannot determine home directory")?
    };

    // /attend skill
    let protocol =
        include_str!("../../messages/narration_protocol.md").replace("{start_skill}", "/attend");
    let skill_content = format!(
        "{}{}",
        format_args!(
            include_str!("../messages/skill_frontmatter.md"),
            skill_name = "attend",
            bin_cmd = bin_cmd,
        ),
        format_args!(
            include_str!("../messages/skill_body.md"),
            bin_cmd = bin_cmd,
            stop_skill = "/unattend",
            narration_protocol = protocol,
        ),
    );

    let attend_dir = base.join(".claude/skills/attend");
    crate::util::atomic_replace_dir(&attend_dir, &[("SKILL.md", &skill_content)])?;

    // /unattend skill (frontmatter only — the UserPromptSubmit hook
    // handles everything; this file just makes /unattend discoverable
    // in the completion list)
    let unattend_content = format!(
        include_str!("../messages/skill_unattend_frontmatter.md"),
        skill_name = "unattend",
    );
    let unattend_dir = base.join(".claude/skills/unattend");
    crate::util::atomic_replace_dir(&unattend_dir, &[("SKILL.md", &unattend_content)])?;

    Ok(())
}
