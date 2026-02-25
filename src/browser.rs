mod chrome;
mod firefox;

/// A browser integration that can install/uninstall native messaging manifests.
pub trait Browser: Sync {
    /// CLI name (e.g., "firefox").
    fn name(&self) -> &'static str;
    /// Install the native messaging host manifest and print extension instructions.
    fn install(&self, bin_cmd: &str) -> anyhow::Result<()>;
    /// Remove the native messaging host manifest.
    fn uninstall(&self) -> anyhow::Result<()>;
}

/// All registered browser backends.
pub const BROWSERS: &[&'static dyn Browser] = &[&chrome::Chrome, &firefox::Firefox];

/// Look up a browser by CLI name.
pub fn browser_by_name(name: &str) -> Option<&'static dyn Browser> {
    BROWSERS.iter().find(|b| b.name() == name).copied()
}
