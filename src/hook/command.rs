use super::types::{HookInput, HookKind};

/// Check if the tool being invoked is `<binary> listen`.
///
/// Matches against the current executable name rather than hardcoding
/// "attend", since the binary may be installed under a different path.
pub(super) fn is_attend_listen(input: &HookInput) -> bool {
    let HookKind::ToolUse {
        bash_command: Some(ref cmd),
    } = input.kind
    else {
        return false;
    };
    let bin_name = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|f| f.to_string_lossy().into_owned()));
    let Some(bin_name) = bin_name else {
        return false;
    };
    is_listen_command(cmd, &bin_name)
}

/// Check whether `cmd` is a `<bin_name> listen [flags]` invocation.
///
/// The command token may be a bare name or a full path; only the
/// filename component is compared against `bin_name`.
pub(super) fn is_listen_command(cmd: &str, bin_name: &str) -> bool {
    let mut parts = cmd.split_whitespace();
    let Some(cmd_bin) = parts.next() else {
        return false;
    };
    let Some(subcmd) = parts.next() else {
        return false;
    };
    let cmd_bin_name = std::path::Path::new(cmd_bin)
        .file_name()
        .map(|f| f.to_string_lossy());
    cmd_bin_name.as_deref() == Some(bin_name) && subcmd == "listen"
}

/// Check if the user prompt is `/attend`.
pub(super) fn is_attend_prompt(input: &HookInput) -> bool {
    matches!(
        &input.kind,
        HookKind::UserPrompt { prompt: Some(p) } if p.trim() == "/attend"
    )
}
