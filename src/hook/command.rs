use super::types::{HookInput, HookKind};

/// What kind of `attend listen` invocation was detected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ListenCommand {
    /// `attend listen` (start/wait mode).
    Listen,
    /// `attend listen --stop` (deactivation).
    ListenStop,
}

/// Detect whether the tool being invoked is `<binary> listen [--stop]`.
///
/// Matches against the current executable name rather than hardcoding
/// "attend", since the binary may be installed under a different path.
pub(super) fn detect_listen_command(input: &HookInput) -> Option<ListenCommand> {
    let HookKind::ToolUse {
        bash_command: Some(ref cmd),
    } = input.kind
    else {
        return None;
    };
    let bin_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()));
    let bin_name = bin_name.as_deref()?;
    parse_listen_command(cmd, bin_name)
}

/// Parse a command string for `<bin_name> listen [--stop]`.
///
/// The command token may be a bare name or a full path; only the
/// filename component is compared against `bin_name`.
pub(super) fn parse_listen_command(cmd: &str, bin_name: &str) -> Option<ListenCommand> {
    let mut parts = cmd.split_whitespace();
    let cmd_bin = parts.next()?;
    let subcmd = parts.next()?;

    let cmd_bin_name = std::path::Path::new(cmd_bin)
        .file_name()
        .map(|f| f.to_string_lossy());
    if cmd_bin_name.as_deref() != Some(bin_name) || subcmd != "listen" {
        return None;
    }

    // Check remaining args for --stop.
    if parts.any(|arg| arg == "--stop") {
        Some(ListenCommand::ListenStop)
    } else {
        Some(ListenCommand::Listen)
    }
}

/// Check if the user prompt is `/attend` (manual install) or `/attend:start` (plugin).
pub(super) fn is_attend_prompt(input: &HookInput) -> bool {
    matches!(
        &input.kind,
        HookKind::UserPrompt { prompt: Some(p) }
            if matches!(p.trim(), "/attend" | "/attend:start")
    )
}

/// Check if the user prompt is `/unattend` (manual install) or `/attend:stop` (plugin).
pub(super) fn is_unattend_prompt(input: &HookInput) -> bool {
    matches!(
        &input.kind,
        HookKind::UserPrompt { prompt: Some(p) }
            if matches!(p.trim(), "/unattend" | "/attend:stop")
    )
}
