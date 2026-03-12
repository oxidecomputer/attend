//! Fish shell integration: preexec/postexec hooks and completions.

use std::fs;

use camino::Utf8PathBuf;
use clap::CommandFactory;

/// Fish hook file installed to `~/.config/fish/conf.d/attend.fish`.
fn hook_path() -> Option<Utf8PathBuf> {
    Some(super::xdg_config_home()?.join("fish/conf.d/attend.fish"))
}

/// Fish completions file installed to `~/.config/fish/completions/attend.fish`.
fn completions_path() -> Option<Utf8PathBuf> {
    Some(super::xdg_config_home()?.join("fish/completions/attend.fish"))
}

pub struct Fish;

impl super::Shell for Fish {
    fn name(&self) -> &'static str {
        "fish"
    }

    fn install_hooks(&self, bin_cmd: &str) -> anyhow::Result<()> {
        let path =
            hook_path().ok_or_else(|| anyhow::anyhow!("cannot determine fish config directory"))?;
        let abs_bin = super::resolve_bin(bin_cmd)?;
        let lock_path = crate::narrate::record_lock_path();

        let script = format!(
            r#"# Installed by attend. Do not edit; reinstall with: attend install --shell fish

function __attend_preexec --on-event fish_preexec
    # Only invoke the binary if the record lock exists (fast path).
    test -f {lock_path}; or return
    command {bin} shell-hook preexec --shell fish --command "$argv"
end

function __attend_postexec --on-event fish_postexec
    set -l __attend_status $status
    set -l __attend_duration $CMD_DURATION
    test -f {lock_path}; or return
    command {bin} shell-hook postexec \
        --shell fish \
        --command "$argv" \
        --exit-status $__attend_status \
        --duration (math "$__attend_duration / 1000")
end
"#,
            lock_path = lock_path,
            bin = abs_bin.display(),
        );

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        crate::util::atomic_write_str(&path, &script)?;

        println!("Installed fish hooks to {path}");

        Ok(())
    }

    fn uninstall_hooks(&self) -> anyhow::Result<()> {
        if let Some(path) = hook_path()
            && path.exists()
        {
            fs::remove_file(&path)?;
            println!("Removed fish hooks from {path}");
        }
        Ok(())
    }

    fn install_completions(&self, _bin_cmd: &str) -> anyhow::Result<()> {
        let path = completions_path()
            .ok_or_else(|| anyhow::anyhow!("cannot determine fish completions directory"))?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        // Generate fish completions via clap_complete.
        let mut buf = Vec::new();
        clap_complete::generate(
            clap_complete::Shell::Fish,
            &mut crate::cli::Cli::command(),
            "attend",
            &mut buf,
        );
        crate::util::atomic_write_str(&path, &String::from_utf8_lossy(&buf))?;

        println!("Installed fish completions to {path}");

        Ok(())
    }

    fn uninstall_completions(&self) -> anyhow::Result<()> {
        if let Some(path) = completions_path()
            && path.exists()
        {
            fs::remove_file(&path)?;
            println!("Removed fish completions from {path}");
        }
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
