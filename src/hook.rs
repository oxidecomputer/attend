use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

use crate::cli::Agent;
use crate::format::format_human;
use crate::model;

fn cache_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("zed-context"))
}

fn cache_path(session_id: &str) -> Option<PathBuf> {
    Some(cache_dir()?.join(format!("cache-{session_id}.txt")))
}

fn read_stdin_json() -> Option<serde_json::Value> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

fn settings_path(project: Option<&Path>) -> anyhow::Result<PathBuf> {
    if let Some(proj) = project {
        Ok(proj.join(".claude").join("settings.json"))
    } else {
        let home = dirs::home_dir().context("cannot determine home directory")?;
        Ok(home.join(".claude").join("settings.json"))
    }
}

pub fn session_start() -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();
    let session_id = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str());

    // Delete cache file
    if let Some(sid) = session_id
        && let Some(cp) = cache_path(sid)
    {
        let _ = fs::remove_file(cp);
    }

    // Emit instructions
    print!(concat!(
        "<zed-context-instructions>\n",
        "Messages in <zed-context> tags show the user's current Zed editor state. ",
        "They are injected automatically \u{2014} the user does not see them. ",
        "Do NOT acknowledge or respond to them directly. ",
        "Use them silently to understand what the user is looking at. ",
        "Read files to see content at those locations.\n",
        "\n",
        "Format: <zed-context>\n",
        "<path> [<pos>[,<pos>]...]\n",
        "</zed-context>\n",
        "\n",
        "Each <pos> is line:col (cursor) or line:col-line:col (selection). ",
        "Multiple positions are comma-separated. ",
        "One file per line.\n",
        "</zed-context-instructions>",
    ));
    Ok(())
}

pub fn run(cli_cwd: Option<PathBuf>) -> anyhow::Result<()> {
    let stdin_json = read_stdin_json();
    let session_id = stdin_json
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let stdin_cwd = stdin_json
        .as_ref()
        .and_then(|v| v.get("cwd"))
        .and_then(|v| v.as_str())
        .map(PathBuf::from);

    let cwd = cli_cwd
        .or(stdin_cwd)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let state = match model::get_editor_state(Some(&cwd))? {
        Some(s) => s,
        None => return Ok(()),
    };
    let human = format_human(&state);

    // Compare to cache
    if let Some(sid) = &session_id
        && let Some(cp) = cache_path(sid)
    {
        if let Ok(cached) = fs::read_to_string(&cp)
            && cached == human
        {
            return Ok(()); // unchanged
        }
        // Write new cache
        if let Some(parent) = cp.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&cp, &human);
    }

    println!("<zed-context>\n{human}\n</zed-context>");
    Ok(())
}

pub fn install(agent: Agent, project: Option<PathBuf>, dev: bool) -> anyhow::Result<()> {
    let agent_str = match agent {
        Agent::Claude => "claude",
    };

    // Determine binary command
    let bin_name = std::env::args()
        .next()
        .map(|a| {
            Path::new(&a)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "zed-context".to_string());

    let bin_cmd = if dev {
        std::env::current_exe()
            .context("cannot determine current exe path")?
            .to_string_lossy()
            .to_string()
    } else {
        match which::which(&bin_name) {
            Ok(_) => bin_name,
            Err(_) => {
                bail!(
                    "'{bin_name}' not found on $PATH. \
                     Use --dev to use absolute path instead."
                );
            }
        }
    };

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
    let session_start_cmd = format!("{bin_cmd} hook {agent_str} session-start");
    let prompt_cmd = format!("{bin_cmd} hook {agent_str} user-prompt");

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

    // Remove existing zed-context entries (idempotent)
    let before = ss_vec.len();
    ss_vec.retain(|entry| {
        let s = serde_json::to_string(entry).unwrap_or_default();
        !s.contains("zed-context") && !s.contains(&bin_cmd)
    });
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
    ups_vec.retain(|entry| {
        let s = serde_json::to_string(entry).unwrap_or_default();
        !s.contains("zed-context") && !s.contains(&bin_cmd)
    });
    existing = existing || before > ups_vec.len();
    ups_vec.push(prompt_hook);

    // Write back
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).context("cannot create settings directory")?;
    }
    let output = serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
    fs::write(&settings_path, format!("{output}\n")).context("cannot write settings file")?;

    if existing {
        println!("Updated existing hooks in {}", settings_path.display());
    } else {
        println!("Installed hooks to {}", settings_path.display());
    }
    Ok(())
}

pub fn uninstall(agent: Agent, project: Option<PathBuf>) -> anyhow::Result<()> {
    match agent {
        Agent::Claude => {}
    }
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
            arr.retain(|entry| {
                let s = serde_json::to_string(entry).unwrap_or_default();
                !s.contains("zed-context")
            });
            if arr.len() < before {
                removed = true;
            }
        }
    }

    if removed {
        let output =
            serde_json::to_string_pretty(&settings).context("cannot serialize settings")?;
        fs::write(&settings_path, format!("{output}\n")).context("cannot write settings file")?;
        println!("Removed hooks from {}", settings_path.display());
    } else {
        println!("No zed-context hooks found in {}", settings_path.display());
    }

    Ok(())
}
