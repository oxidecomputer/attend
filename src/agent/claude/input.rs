use std::io::{self, Read};

use camino::Utf8PathBuf;
use serde::Deserialize;

use crate::hook::{HookInput, HookKind, HookType};
use crate::state::SessionId;

/// Raw JSON payload from Claude Code hook stdin.
///
/// All fields are optional/defaulted because different hook types populate
/// different subsets. The `HookType` discriminant (passed to `parse`)
/// determines which fields are used to construct the typed `HookKind`.
#[derive(Deserialize, Default)]
struct ClaudeHookStdin {
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    cwd: Option<Utf8PathBuf>,
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    stop_hook_active: bool,
    #[serde(default)]
    tool_name: Option<String>,
    #[serde(default)]
    tool_input: Option<ToolInput>,
}

/// Typed subset of the `tool_input` object we care about.
///
/// Claude Code passes the full tool invocation payload; we only need
/// the `command` field (present when `tool_name == "Bash"`).
#[derive(Deserialize, Default)]
struct ToolInput {
    #[serde(default)]
    command: Option<String>,
}

/// Parse Claude Code hook stdin JSON into a typed `HookInput`.
pub(super) fn parse(hook_type: HookType) -> HookInput {
    let mut buf = String::new();
    let _ = io::stdin().read_to_string(&mut buf);
    let raw: ClaudeHookStdin = serde_json::from_str(&buf).unwrap_or_default();
    let kind = match hook_type {
        HookType::SessionStart => HookKind::SessionStart,
        HookType::UserPrompt => HookKind::UserPrompt { prompt: raw.prompt },
        HookType::Stop => HookKind::Stop {
            stop_hook_active: raw.stop_hook_active,
        },
        HookType::ToolUse => {
            let bash_command = raw
                .tool_name
                .as_deref()
                .filter(|n| *n == "Bash")
                .and_then(|_| raw.tool_input.and_then(|ti| ti.command));
            HookKind::ToolUse { bash_command }
        }
    };
    HookInput {
        session_id: raw.session_id.map(SessionId::from),
        cwd: raw.cwd,
        kind,
    }
}
