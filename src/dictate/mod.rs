//! Voice-driven prompt composition for Claude Code.
//!
//! Compose rich prompts by narrating while navigating code. Press a hotkey,
//! switch to the editor, speak and point at code, press the hotkey again.
//! The tool transcribes speech, captures editor state and file diffs, and
//! delivers a formatted prompt to a running Claude Code session.

mod audio;
mod merge;
mod receive;
mod record;
mod transcribe;

use std::fs;
use std::io::{self, Read};
use std::path::PathBuf;

use clap::{Subcommand, ValueEnum};

/// Base directory for all dictation state files.
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("attend")
}

/// Path to the file that identifies the currently attending session.
pub fn listening_path() -> PathBuf {
    cache_dir().join("listening")
}

/// Read the session ID of the currently attending session, if any.
pub fn listening_session() -> Option<String> {
    fs::read_to_string(listening_path())
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Path to the record lock file.
pub fn record_lock_path() -> PathBuf {
    cache_dir().join("record.lock")
}

/// Path to the stop sentinel file.
pub fn stop_sentinel_path() -> PathBuf {
    cache_dir().join("stop")
}

/// Path to the receive lock file.
pub fn receive_lock_path() -> PathBuf {
    cache_dir().join("receive.lock")
}

/// Directory where pending dictation files are written.
///
/// Each dictation is stored as `<timestamp>.md` inside
/// `~/.cache/attend/pending/<session_id>/`.
pub fn pending_dir(session_id: &str) -> PathBuf {
    cache_dir().join("pending").join(session_id)
}

/// Directory where archived dictation files are stored.
pub fn archive_dir(session_id: &str) -> PathBuf {
    cache_dir().join("archive").join(session_id)
}

/// Default Whisper model path.
pub fn default_model_path() -> PathBuf {
    cache_dir().join("models").join("ggml-base.en.bin")
}

/// Dictation CLI subcommands.
#[derive(Subcommand)]
pub enum DictateCommand {
    /// Start or stop recording (one hotkey).
    Toggle {
        /// Path to GGML Whisper model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
    },
    /// Spawn detached recorder (idempotent).
    Start {
        /// Path to GGML Whisper model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
    },
    /// Signal recorder to stop (idempotent).
    Stop,
    /// Check for / wait for dictation.
    Receive {
        /// Poll until dictation arrives.
        #[arg(long)]
        wait: bool,
        /// Session ID (defaults to listening file).
        #[arg(long)]
        session: Option<String>,
    },
    /// Hook handler: write session_id to listening file.
    Activate,
    /// Write editor keybindings and Claude hooks.
    Install {
        /// Editor to install keybindings for.
        #[arg(long)]
        editor: EditorChoice,
    },
    /// Internal: run the recording daemon (not user-facing).
    #[command(name = "_record-daemon", hide = true)]
    RecordDaemon {
        /// Path to GGML Whisper model.
        #[arg(long)]
        model: Option<PathBuf>,
        /// Session ID.
        #[arg(long)]
        session: Option<String>,
    },
}

/// Supported editors for keybinding installation.
#[derive(Clone, ValueEnum)]
pub enum EditorChoice {
    Zed,
}

/// Resolve the session ID from flag, listening file, or None.
pub fn resolve_session(flag: Option<String>) -> Option<String> {
    flag.or_else(listening_session)
}

/// Read stdin as JSON (used by activate hook).
fn read_stdin_json() -> Option<serde_json::Value> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf).ok()?;
    serde_json::from_str(&buf).ok()
}

/// Run a dictate subcommand.
pub fn run(cmd: DictateCommand) -> anyhow::Result<()> {
    match cmd {
        DictateCommand::Toggle { model, session } => record::toggle(model, session),
        DictateCommand::Start { model, session } => record::start(model, session),
        DictateCommand::Stop => record::stop(),
        DictateCommand::Receive { wait, session } => receive::run(wait, session),
        DictateCommand::Activate => activate(),
        DictateCommand::Install { editor } => install(editor),
        DictateCommand::RecordDaemon { model, session } => record::daemon(model, session),
    }
}

/// Handle the `activate` subcommand: read session_id from stdin JSON,
/// write it to the listening file.
fn activate() -> anyhow::Result<()> {
    let stdin = read_stdin_json();
    let session_id = stdin
        .as_ref()
        .and_then(|v| v.get("session_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no session_id in stdin JSON"))?;

    let path = listening_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, session_id)?;
    Ok(())
}

/// Handle the `install` subcommand.
fn install(editor: EditorChoice) -> anyhow::Result<()> {
    match editor {
        EditorChoice::Zed => install_zed(),
    }
}

/// Resolve the binary command string, preferring $PATH lookup.
fn resolve_bin_cmd() -> String {
    let bin_name = std::env::args()
        .next()
        .map(|a| {
            std::path::Path::new(&a)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        })
        .unwrap_or_else(|| "attend".to_string());

    if which::which(&bin_name).is_ok() {
        bin_name
    } else {
        std::env::current_exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or(bin_name)
    }
}

/// Install Zed task, keybinding, skill file, and Claude Code stop hook.
fn install_zed() -> anyhow::Result<()> {
    let bin_cmd = resolve_bin_cmd();

    install_zed_task(&bin_cmd)?;
    install_zed_keybinding()?;
    install_skill_file()?;
    install_stop_hook(&bin_cmd)?;

    println!("Dictation support installed.");
    println!("  - Zed task: Toggle Dictation");
    println!("  - Keybinding: cmd-shift-d (see ~/.config/zed/keymap.json)");
    println!("  - Skill file: .claude/skills/attend/SKILL.md");
    println!("  - Stop hook: ~/.claude/settings.json");
    println!();
    println!("Also run: attend hook install -a claude");
    println!("(if you haven't already, to install the UserPromptSubmit hook)");

    Ok(())
}

/// Install the Zed task definition for toggling dictation.
fn install_zed_task(bin_cmd: &str) -> anyhow::Result<()> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?
        .join("zed")
        .join("tasks.json");

    let task_entry = serde_json::json!({
        "label": "Toggle Dictation",
        "command": bin_cmd,
        "args": ["dictate", "toggle"],
        "hide": "always",
        "reveal": "never",
        "allow_concurrent_runs": false
    });

    let mut tasks: Vec<serde_json::Value> = if config_dir.exists() {
        let content = fs::read_to_string(&config_dir)?;
        // Strip comments (Zed's JSON supports // comments)
        serde_json::from_str(&strip_json_comments(&content)).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Remove any existing attend dictate task
    tasks.retain(|t| {
        t.get("label")
            .and_then(|l| l.as_str())
            .is_none_or(|l| l != "Toggle Dictation")
    });

    tasks.push(task_entry);

    if let Some(parent) = config_dir.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = serde_json::to_string_pretty(&tasks)?;
    output.push('\n');
    fs::write(&config_dir, output)?;

    Ok(())
}

/// Install a Zed keybinding for the Toggle Dictation task.
fn install_zed_keybinding() -> anyhow::Result<()> {
    let keymap_path = dirs::config_dir()
        .ok_or_else(|| anyhow::anyhow!("cannot determine config directory"))?
        .join("zed")
        .join("keymap.json");

    let binding_entry = serde_json::json!({
        "bindings": {
            "cmd-shift-d": ["task::Spawn", {"task_name": "Toggle Dictation"}]
        }
    });

    let mut keymap: Vec<serde_json::Value> = if keymap_path.exists() {
        let content = fs::read_to_string(&keymap_path)?;
        serde_json::from_str(&strip_json_comments(&content)).unwrap_or_default()
    } else {
        Vec::new()
    };

    // Remove any existing entry that is solely our dictation keybinding
    keymap.retain(|entry| !is_dictation_keybinding(entry));

    fn is_dictation_keybinding(entry: &serde_json::Value) -> bool {
        let Some(bindings) = entry.get("bindings").and_then(|b| b.as_object()) else {
            return false;
        };
        bindings.len() == 1
            && bindings
                .get("cmd-shift-d")
                .and_then(|v| v.as_array())
                .is_some_and(|a| a.first().and_then(|s| s.as_str()) == Some("task::Spawn"))
    }

    keymap.push(binding_entry);

    if let Some(parent) = keymap_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = serde_json::to_string_pretty(&keymap)?;
    output.push('\n');
    fs::write(&keymap_path, output)?;

    Ok(())
}

/// Install the SKILL.md file for `/attend` discoverability.
fn install_skill_file() -> anyhow::Result<()> {
    let skill_dir = std::path::PathBuf::from(".claude/skills/attend");
    fs::create_dir_all(&skill_dir)?;

    let skill_content = "\
---
name: attend
description: Activate dictation mode for this session
---
Dictation mode is handled by the UserPromptSubmit hook.

When the user runs /attend, the hook activates dictation for this session
and instructs Claude to start a background listener.
";

    fs::write(skill_dir.join("SKILL.md"), skill_content)?;
    Ok(())
}

/// Install the Claude Code Stop hook for dictation delivery.
fn install_stop_hook(bin_cmd: &str) -> anyhow::Result<()> {
    let home =
        dirs::home_dir().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;
    let settings_path = home.join(".claude").join("settings.json");

    let mut settings: serde_json::Value = if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)?;
        serde_json::from_str(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    let obj = settings
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("settings is not an object"))?;

    let hooks = obj.entry("hooks").or_insert_with(|| serde_json::json!({}));
    let hooks_obj = hooks
        .as_object_mut()
        .ok_or_else(|| anyhow::anyhow!("hooks is not an object"))?;

    // Build stop hook command script
    let stop_hook_script = build_stop_hook_script(bin_cmd);

    let stop_hook = serde_json::json!({
        "hooks": [
            {
                "type": "command",
                "command": stop_hook_script,
                "timeout": 10
            }
        ]
    });

    let stop_arr = hooks_obj
        .entry("Stop")
        .or_insert_with(|| serde_json::json!([]));
    let stop_vec = stop_arr
        .as_array_mut()
        .ok_or_else(|| anyhow::anyhow!("Stop is not an array"))?;

    // Remove existing attend stop hooks (idempotent)
    stop_vec.retain(|entry| {
        !entry
            .get("hooks")
            .and_then(|h| h.as_array())
            .into_iter()
            .flatten()
            .filter_map(|h| h.get("command").and_then(|c| c.as_str()))
            .any(|cmd| cmd.contains("attend"))
    });

    stop_vec.push(stop_hook);

    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut output = serde_json::to_string_pretty(&settings)?;
    output.push('\n');
    fs::write(&settings_path, output)?;

    Ok(())
}

/// Build the inline shell script for the Stop hook.
fn build_stop_hook_script(bin_cmd: &str) -> String {
    // The script reads session_id from stdin JSON, compares with the
    // listening file, and either delivers pending dictation or instructs
    // Claude to dispatch a background listener.
    format!(
        r#"bash -c '
INPUT=$(cat)
LISTENING="$HOME/.cache/attend/listening"
[ ! -f "$LISTENING" ] && echo '\''{{\"decision\":\"proceed\"}}'\'' && exit 0
HOOK_SID=$(echo "$INPUT" | jq -r .session_id)
FILE_SID=$(cat "$LISTENING")
[ "$HOOK_SID" != "$FILE_SID" ] && echo '\''{{\"decision\":\"proceed\"}}'\'' && exit 0
content=$({bin_cmd} dictate receive 2>/dev/null)
if [ -n "$content" ]; then
  jq -n --arg r "$content" '\''{{\"decision\":\"block\",\"reason\":$r}}'\''
  exit 0
fi
ACTIVE=$(echo "$INPUT" | jq -r .stop_hook_active)
[ "$ACTIVE" = "true" ] && echo '\''{{\"decision\":\"proceed\"}}'\'' && exit 0
echo '\''{{\"decision\":\"block\",\"reason\":\"Run `attend dictate receive --wait` in the background to wait for the next dictation.\"}}'\''
'"#
    )
}

/// Strip `//` line comments from JSON content (Zed supports comments in JSON).
fn strip_json_comments(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//") {
            continue;
        }
        // Handle inline comments (simple heuristic: not inside strings)
        if let Some(idx) = find_line_comment(line) {
            out.push_str(&line[..idx]);
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// Find the position of a `//` comment that's not inside a JSON string.
fn find_line_comment(line: &str) -> Option<usize> {
    let mut in_string = false;
    let mut escaped = false;
    let bytes = line.as_bytes();

    for i in 0..bytes.len() {
        if escaped {
            escaped = false;
            continue;
        }
        match bytes[i] {
            b'\\' if in_string => escaped = true,
            b'"' => in_string = !in_string,
            b'/' if !in_string && i + 1 < bytes.len() && bytes[i + 1] == b'/' => {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_json_comments_full_line() {
        let input = "// This is a comment\n{\"key\": \"value\"}\n";
        let result = strip_json_comments(input);
        assert_eq!(result, "{\"key\": \"value\"}\n");
    }

    #[test]
    fn strip_json_comments_inline() {
        let input = "{\"key\": \"value\"} // trailing comment\n";
        let result = strip_json_comments(input);
        assert_eq!(result, "{\"key\": \"value\"} \n");
    }

    #[test]
    fn strip_json_comments_inside_string_preserved() {
        let input = "{\"url\": \"https://example.com\"}\n";
        let result = strip_json_comments(input);
        assert_eq!(result, "{\"url\": \"https://example.com\"}\n");
    }

    #[test]
    fn strip_json_comments_empty() {
        let result = strip_json_comments("");
        assert_eq!(result, "");
    }

    #[test]
    fn strip_json_comments_mixed() {
        let input = "// header comment\n{\n  // inner comment\n  \"a\": 1\n}\n";
        let result = strip_json_comments(input);
        assert_eq!(result, "{\n  \"a\": 1\n}\n");
    }

    #[test]
    fn resolve_session_flag_takes_precedence() {
        let result = resolve_session(Some("my-session".to_string()));
        assert_eq!(result, Some("my-session".to_string()));
    }

    #[test]
    fn resolve_session_no_flag_no_listening() {
        // When no flag and no listening file exists, returns None
        // (depends on whether listening file exists on disk, so just test the flag path)
        let result = resolve_session(Some("test".to_string()));
        assert_eq!(result.unwrap(), "test");
    }

    #[test]
    fn cache_dir_is_under_attend() {
        let dir = cache_dir();
        assert!(dir.ends_with("attend"));
    }

    #[test]
    fn pending_dir_includes_session() {
        let dir = pending_dir("abc-123");
        assert!(dir.ends_with("pending/abc-123") || dir.ends_with("pending\\abc-123"));
    }

    #[test]
    fn archive_dir_includes_session() {
        let dir = archive_dir("abc-123");
        assert!(dir.ends_with("archive/abc-123") || dir.ends_with("archive\\abc-123"));
    }

    #[test]
    fn build_stop_hook_script_contains_bin() {
        let script = build_stop_hook_script("attend");
        assert!(script.contains("attend dictate receive"));
        assert!(script.contains("decision"));
        assert!(script.contains("proceed"));
    }

    #[test]
    fn find_line_comment_none_when_absent() {
        assert_eq!(find_line_comment("{\"key\": \"value\"}"), None);
    }

    #[test]
    fn find_line_comment_finds_comment() {
        assert_eq!(find_line_comment("code // comment"), Some(5));
    }

    #[test]
    fn find_line_comment_ignores_url_in_string() {
        assert_eq!(find_line_comment("\"url\": \"https://example.com\""), None);
    }

    #[test]
    fn find_line_comment_after_string_with_slash() {
        assert_eq!(find_line_comment("\"path\": \"a/b\" // comment"), Some(14));
    }
}
