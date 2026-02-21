use super::*;

/// Install hooks into Claude Code settings.
pub fn install(bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
    let settings_path = settings_path(project.as_deref().map(|p| p.as_std_path()))?;

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
    let session_start_cmd = format!("{bin_cmd} hook session-start --agent claude");
    let prompt_cmd = format!("{bin_cmd} hook user-prompt --agent claude");

    // Build the hooks structure
    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks.as_object_mut().context("hooks is not an object")?;

    // SessionStart
    let session_start_hook = serde_json::json!({
        HOOK_MARKER_KEY: HOOK_MARKER_VALUE,
        "matcher": "startup|clear|compact",
        "hooks": [
            {
                "type": "command",
                "command": session_start_cmd
            }
        ]
    });

    let ss_arr = hooks_obj
        .entry(HOOK_KEY_SESSION_START)
        .or_insert_with(|| serde_json::json!([]));
    let ss_vec = ss_arr
        .as_array_mut()
        .context("SessionStart is not an array")?;

    // Remove existing attend entries (idempotent)
    let before = ss_vec.len();
    ss_vec.retain(|entry| !is_our_hook(entry));
    let mut existing = before > ss_vec.len();
    ss_vec.push(session_start_hook);

    // UserPromptSubmit
    let prompt_hook = serde_json::json!({
        HOOK_MARKER_KEY: HOOK_MARKER_VALUE,
        "hooks": [
            {
                "type": "command",
                "command": prompt_cmd,
                "timeout": 5
            }
        ]
    });

    let ups_arr = hooks_obj
        .entry(HOOK_KEY_USER_PROMPT_SUBMIT)
        .or_insert_with(|| serde_json::json!([]));
    let ups_vec = ups_arr
        .as_array_mut()
        .context("UserPromptSubmit is not an array")?;

    let before = ups_vec.len();
    ups_vec.retain(|entry| !is_our_hook(entry));
    existing = existing || before > ups_vec.len();
    ups_vec.push(prompt_hook);

    // Narration delivery hooks: Stop, PreToolUse, PostToolUse.
    // All three run the same check-and-deliver logic. PreToolUse and
    // PostToolUse ensure narration arrives between tools within a single
    // response, not just at the end.
    for (key, subcommand) in [
        (HOOK_KEY_STOP, "stop"),
        (HOOK_KEY_PRE_TOOL_USE, "pre-tool-use"),
        (HOOK_KEY_POST_TOOL_USE, "post-tool-use"),
    ] {
        let cmd = format!("{bin_cmd} hook {subcommand} --agent claude");
        let hook = serde_json::json!({
            HOOK_MARKER_KEY: HOOK_MARKER_VALUE,
            "hooks": [
                {
                    "type": "command",
                    "command": cmd,
                    "timeout": 10
                }
            ]
        });

        let arr = hooks_obj
            .entry(key)
            .or_insert_with(|| serde_json::json!([]));
        let vec = arr
            .as_array_mut()
            .context(format!("{key} is not an array"))?;

        let before = vec.len();
        vec.retain(|entry| !is_our_hook(entry));
        existing = existing || before > vec.len();
        vec.push(hook);
    }

    // Pre-authorize `attend look` so Claude doesn't prompt for every look call.
    {
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
        // Remove our own entries, then re-add current.
        // Match on the exact patterns we install, not a substring search,
        // to avoid clobbering unrelated permissions.
        allow_vec.retain(|v: &serde_json::Value| {
            v.as_str()
                .map(|s| s != look_pattern && s != listen_pattern)
                .unwrap_or(true)
        });
        allow_vec.push(serde_json::Value::String(look_pattern));
        allow_vec.push(serde_json::Value::String(listen_pattern));
    }

    // Write back
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).context("cannot create settings directory")?;
    }
    let mut output =
        serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
    output.push('\n');
    crate::util::atomic_write_str(&settings_path, &output)
        .map_err(|e| anyhow::anyhow!("cannot write settings file: {e}"))?;

    // SKILL.md for /attend discoverability
    install_skill_file(bin_cmd, project.as_deref().map(|p| p.as_std_path()))?;

    if existing {
        println!("Updated existing hooks in {}", settings_path.display());
    } else {
        println!("Installed hooks to {}", settings_path.display());
    }
    Ok(())
}

/// Install the SKILL.md file for `/attend` discoverability.
fn install_skill_file(bin_cmd: &str, project: Option<&Path>) -> anyhow::Result<()> {
    let base = if let Some(proj) = project {
        proj.to_path_buf()
    } else {
        dirs::home_dir().context("cannot determine home directory")?
    };
    let skill_dir = base.join(".claude/skills/attend");
    fs::create_dir_all(&skill_dir)?;

    let protocol = include_str!("../../messages/narration_protocol.md");
    let skill_content = format!(
        "{}{}",
        format_args!(
            include_str!("../messages/skill_frontmatter.md"),
            bin_cmd = bin_cmd
        ),
        format_args!(
            include_str!("../messages/skill_body.md"),
            bin_cmd = bin_cmd,
            narration_protocol = protocol,
        ),
    );

    crate::util::atomic_write_str(skill_dir.join("SKILL.md"), &skill_content)?;
    Ok(())
}
