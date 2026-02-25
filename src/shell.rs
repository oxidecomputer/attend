use camino::Utf8PathBuf;

mod fish;
mod zsh;

/// XDG config home: `$XDG_CONFIG_HOME` if set, otherwise `~/.config`.
///
/// Fish and zsh both use XDG paths, and `dirs::config_dir()` returns
/// `~/Library/Application Support` on macOS — wrong for shell config.
fn xdg_config_home() -> Option<Utf8PathBuf> {
    if let Ok(val) = std::env::var("XDG_CONFIG_HOME")
        && !val.is_empty()
    {
        return Some(Utf8PathBuf::from(val));
    }
    let home = dirs::home_dir()?;
    let home = Utf8PathBuf::try_from(home).ok()?;
    Some(home.join(".config"))
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
