use std::path::PathBuf;

use camino::Utf8PathBuf;

mod fish;
mod zsh;

/// Resolve the absolute path to the attend binary for hook scripts.
pub(crate) fn resolve_bin(bin_cmd: &str) -> anyhow::Result<PathBuf> {
    if std::path::Path::new(bin_cmd).is_absolute() {
        Ok(bin_cmd.into())
    } else {
        which::which(bin_cmd).map_err(|e| anyhow::anyhow!("cannot find {bin_cmd} on PATH: {e}"))
    }
}

/// XDG config home: delegates to [`crate::util::xdg_config_home`].
fn xdg_config_home() -> Option<Utf8PathBuf> {
    crate::util::xdg_config_home()
}

/// A shell integration that can install/uninstall hooks and completions.
pub trait Shell: Sync {
    /// CLI name (e.g., "fish", "zsh").
    fn name(&self) -> &'static str;

    /// Install shell hooks for narration capture.
    ///
    /// Writes hook files and prints the user's required config change
    /// (e.g., the `source` line for their rc file).
    fn install_hooks(&self, bin_cmd: &str) -> anyhow::Result<()>;

    /// Remove shell hooks.
    fn uninstall_hooks(&self) -> anyhow::Result<()>;

    /// Install shell completions.
    fn install_completions(&self, bin_cmd: &str) -> anyhow::Result<()>;

    /// Remove shell completions.
    fn uninstall_completions(&self) -> anyhow::Result<()>;

    /// Check the health of the shell integration.
    /// Returns a list of diagnostic warnings (empty = healthy).
    fn check(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// All registered shell backends.
pub const SHELLS: &[&dyn Shell] = &[&fish::Fish, &zsh::Zsh];

/// Look up a shell by CLI name.
pub fn shell_by_name(name: &str) -> Option<&'static dyn Shell> {
    SHELLS.iter().find(|s| s.name() == name).copied()
}
