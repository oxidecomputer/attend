//! Zsh shell integration: preexec/precmd hooks and completions.

use std::fs;

use camino::Utf8PathBuf;
use clap::CommandFactory;

/// Zsh hook file installed to `~/.config/attend/hooks/attend.zsh`.
fn hook_path() -> Option<Utf8PathBuf> {
    Some(super::xdg_config_home()?.join("attend/hooks/attend.zsh"))
}

/// Zsh completions file installed to `~/.config/attend/completions/_attend`.
fn completions_path() -> Option<Utf8PathBuf> {
    Some(super::xdg_config_home()?.join("attend/completions/_attend"))
}

/// Completions directory (for fpath).
fn completions_dir() -> Option<Utf8PathBuf> {
    Some(super::xdg_config_home()?.join("attend/completions"))
}

/// Path to the user's `~/.zshrc`.
fn zshrc_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".zshrc"))
}

/// Marker comment appended to lines we add to `~/.zshrc`.
const HOOKS_MARKER: &str = "# attend:hooks";
const COMPLETIONS_MARKER: &str = "# attend:completions";

/// Append a line to `~/.zshrc` if no line containing `marker` is already
/// present. Creates the file if it doesn't exist.
fn add_to_zshrc(line: &str, marker: &str) -> anyhow::Result<()> {
    let path = zshrc_path().ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?;

    let existing = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        String::new()
    };

    if existing.lines().any(|l| l.contains(marker)) {
        return Ok(()); // already present
    }

    let mut content = existing;
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(line);
    content.push_str("  ");
    content.push_str(marker);
    content.push('\n');

    crate::util::atomic_write_str(&path, &content)?;
    Ok(())
}

/// Remove all lines containing `marker` from `~/.zshrc`.
fn remove_from_zshrc(marker: &str) -> anyhow::Result<()> {
    let path = match zshrc_path() {
        Some(p) if p.exists() => p,
        _ => return Ok(()),
    };

    let content = fs::read_to_string(&path)?;
    let filtered: String = content
        .lines()
        .filter(|line| !line.contains(marker))
        .collect::<Vec<_>>()
        .join("\n");

    // Preserve trailing newline if original had one.
    let filtered = if content.ends_with('\n') && !filtered.is_empty() {
        filtered + "\n"
    } else {
        filtered
    };

    if filtered != content {
        crate::util::atomic_write_str(&path, &filtered)?;
    }

    Ok(())
}

pub struct Zsh;

impl super::Shell for Zsh {
    fn name(&self) -> &'static str {
        "zsh"
    }

    fn install_hooks(&self, bin_cmd: &str) -> anyhow::Result<()> {
        let path = hook_path()
            .ok_or_else(|| anyhow::anyhow!("cannot determine attend config directory"))?;
        let abs_bin = super::resolve_bin(bin_cmd)?;
        let lock_path = crate::narrate::record_lock_path();

        let script = format!(
            r#"# Installed by attend. Do not edit; reinstall with: attend install --shell zsh

__attend_preexec() {{
    # $1 is the command string (from zsh's preexec hook).
    [[ -f {lock_path} ]] || return
    command {bin} shell-hook preexec --shell zsh --command "$1"
}}

__attend_precmd() {{
    local __attend_status=$?
    local __attend_end=$EPOCHREALTIME
    [[ -f {lock_path} ]] || return
    if [[ -n "$__attend_cmd" ]]; then
        local __attend_duration
        __attend_duration=$(( __attend_end - __attend_start ))
        command {bin} shell-hook postexec \
            --shell zsh \
            --command "$__attend_cmd" \
            --exit-status $__attend_status \
            --duration $__attend_duration
        unset __attend_cmd __attend_start
    fi
}}

__attend_record_start() {{
    __attend_cmd="$1"
    __attend_start=$EPOCHREALTIME
}}

autoload -Uz add-zsh-hook
add-zsh-hook preexec __attend_record_start
add-zsh-hook preexec __attend_preexec
add-zsh-hook precmd __attend_precmd
"#,
            lock_path = lock_path,
            bin = abs_bin.display(),
        );

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::util::atomic_write_str(&path, &script)?;

        // Auto-source from ~/.zshrc.
        let source_line = format!("[[ -f {path} ]] && source {path}");
        add_to_zshrc(&source_line, HOOKS_MARKER)?;

        println!("Installed zsh hooks to {path}");

        Ok(())
    }

    fn uninstall_hooks(&self) -> anyhow::Result<()> {
        if let Some(path) = hook_path()
            && path.exists()
        {
            fs::remove_file(&path)?;
            println!("Removed zsh hooks from {path}");
        }
        remove_from_zshrc(HOOKS_MARKER)?;
        Ok(())
    }

    fn install_completions(&self, _bin_cmd: &str) -> anyhow::Result<()> {
        let path = completions_path()
            .ok_or_else(|| anyhow::anyhow!("cannot determine attend completions directory"))?;
        let dir = completions_dir().unwrap();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Generate zsh completions via clap_complete.
        let mut buf = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Zsh,
            &mut crate::cli::Cli::command(),
            "attend",
            &mut buf,
        );
        crate::util::atomic_write_str(&path, &String::from_utf8_lossy(&buf))?;

        // Auto-add fpath to ~/.zshrc.
        let fpath_line = format!("fpath=({dir} $fpath)");
        add_to_zshrc(&fpath_line, COMPLETIONS_MARKER)?;

        println!("Installed zsh completions to {path}");

        Ok(())
    }

    fn uninstall_completions(&self) -> anyhow::Result<()> {
        if let Some(path) = completions_path()
            && path.exists()
        {
            fs::remove_file(&path)?;
            println!("Removed zsh completions from {path}");
        }
        remove_from_zshrc(COMPLETIONS_MARKER)?;
        Ok(())
    }

    fn check(&self) -> anyhow::Result<Vec<String>> {
        let mut warnings = Vec::new();
        if let Some(path) = hook_path()
            && !path.exists()
        {
            warnings.push("hooks not installed".to_string());
        }
        Ok(warnings)
    }
}
