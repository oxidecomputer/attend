use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use camino::Utf8PathBuf;

use super::{Agent, HookEvent};

/// Claude Code hook configuration keys.
const HOOK_KEY_SESSION_START: &str = "SessionStart";
const HOOK_KEY_USER_PROMPT_SUBMIT: &str = "UserPromptSubmit";
const HOOK_KEY_STOP: &str = "Stop";

/// Claude Code agent backend.
pub struct Claude;

impl Agent for Claude {
    fn name(&self) -> &'static str {
        "claude"
    }

    fn run_hook(&self, event: HookEvent, cwd: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        match event {
            HookEvent::SessionStart => crate::hook::session_start(),
            HookEvent::UserPrompt => crate::hook::run(cwd),
            HookEvent::Stop => crate::hook::stop(),
        }
    }

    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        install(bin_cmd, project)
    }

    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        uninstall(project)
    }
}

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

/// Resolve the Claude Code settings file path.
///
/// Global installs use `~/.claude/settings.json`. Project installs use
/// `settings.local.json` so as not to impose the tool on other contributors.
fn settings_path(project: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(proj) = project {
        Ok(proj.join(".claude").join("settings.local.json"))
    } else {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".claude").join("settings.json"))
    }
}

/// Install hooks into Claude Code settings.
fn install(bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
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
        .entry(HOOK_KEY_USER_PROMPT_SUBMIT)
        .or_insert_with(|| serde_json::json!([]));
    let ups_vec = ups_arr
        .as_array_mut()
        .context("UserPromptSubmit is not an array")?;

    let before = ups_vec.len();
    ups_vec.retain(|entry| !entry_has_attend_cmd(entry, Some(bin_cmd)));
    existing = existing || before > ups_vec.len();
    ups_vec.push(prompt_hook);

    // Stop hook (narration delivery)
    {
        let stop_cmd = format!("{bin_cmd} hook stop --agent claude");
        let stop_hook = serde_json::json!({
            "hooks": [
                {
                    "type": "command",
                    "command": stop_cmd,
                    "timeout": 10
                }
            ]
        });

        let stop_arr = hooks_obj
            .entry(HOOK_KEY_STOP)
            .or_insert_with(|| serde_json::json!([]));
        let stop_vec = stop_arr.as_array_mut().context("Stop is not an array")?;

        let before = stop_vec.len();
        stop_vec.retain(|entry| !entry_has_attend_cmd(entry, Some(bin_cmd)));
        existing = existing || before > stop_vec.len();
        stop_vec.push(stop_hook);
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
        // Remove stale attend entries, then add current
        allow_vec.retain(|v: &serde_json::Value| {
            v.as_str().map(|s| !s.contains("attend")).unwrap_or(true)
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

/// Remove hooks from Claude Code settings.
fn uninstall(project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
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
    ];
    for key in &hook_keys {
        if let Some(arr) = hooks.get_mut(*key).and_then(|v| v.as_array_mut()) {
            let before = arr.len();
            arr.retain(|entry| !entry_has_attend_cmd(entry, None));
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

/// Install the SKILL.md file for `/attend` discoverability.
fn install_skill_file(bin_cmd: &str, project: Option<&Path>) -> anyhow::Result<()> {
    let base = if let Some(proj) = project {
        proj.to_path_buf()
    } else {
        dirs::home_dir().context("cannot determine home directory")?
    };
    let skill_dir = base.join(".claude/skills/attend");
    fs::create_dir_all(&skill_dir)?;

    let skill_content = format!(
        "{}{}",
        format_args!(
            include_str!("claude_skill_frontmatter.md"),
            bin_cmd = bin_cmd
        ),
        format_args!(include_str!("claude_skill_body.md"), bin_cmd = bin_cmd),
    );

    crate::util::atomic_write_str(skill_dir.join("SKILL.md"), &skill_content)?;
    Ok(())
}
