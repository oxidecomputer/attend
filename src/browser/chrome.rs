//! Chrome browser integration: native messaging host manifest + unpacked extension.
//!
//! Unlike Firefox, Chrome unpacked extensions persist across restarts (they
//! just show an occasional "developer mode" banner). So install copies the
//! extension files to a persistent local directory and prints load instructions.

use std::path::Path;

use native_messaging::install::{manifest, paths::Scope};

/// Extension ID derived from the public key in `manifest.chrome.json`.
///
/// SHA-256 of the DER public key, first 16 bytes, hex digits mapped a-p.
const EXTENSION_ID: &str = "pmafeboglfakekjeegfabgiibhhijkfo";

/// Native messaging host name (must match the extension's `sendNativeMessage` call).
const HOST_NAME: &str = "attend";

/// Human-readable description for the native messaging manifest.
const DESCRIPTION: &str = "Attend browser bridge: captures selections for narration";

/// Extension files embedded at compile time.
const CONTENT_JS: &str = include_str!("../../extension/content.js");
const BACKGROUND_JS: &str = include_str!("../../extension/background.js");
const MANIFEST_JSON: &str = include_str!("../../extension/manifest.chrome.json");

pub struct Chrome;

impl Chrome {
    /// Directory where the unpacked extension is installed.
    fn extension_dir() -> anyhow::Result<std::path::PathBuf> {
        let data_dir =
            dirs::data_dir().ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
        Ok(data_dir.join("attend").join("chrome-extension"))
    }
}

impl super::Browser for Chrome {
    fn name(&self) -> &'static str {
        "chrome"
    }

    fn install(&self, bin_cmd: &str) -> anyhow::Result<()> {
        let exe_path = Path::new(bin_cmd);

        // Register the native messaging host manifest.
        let origin = format!("chrome-extension://{EXTENSION_ID}/");
        manifest::install(
            HOST_NAME,
            DESCRIPTION,
            exe_path,
            &[origin], // chrome origins
            &[],       // firefox extensions (unused)
            &["chrome"],
            Scope::User,
        )?;

        let ok = manifest::verify_installed(HOST_NAME, Some(&["chrome"]), Scope::User)?;
        if !ok {
            anyhow::bail!("native messaging manifest verification failed");
        }

        // Write the unpacked extension to a persistent directory.
        let ext_dir = Self::extension_dir()?;
        std::fs::create_dir_all(&ext_dir)?;
        std::fs::write(ext_dir.join("manifest.json"), MANIFEST_JSON)?;
        std::fs::write(ext_dir.join("content.js"), CONTENT_JS)?;
        std::fs::write(ext_dir.join("background.js"), BACKGROUND_JS)?;

        println!("Installed native messaging host manifest for Chrome.");
        println!("Extension files written to: {}", ext_dir.display());
        println!();
        println!("Load the extension in Chrome:");
        println!("  1. Open chrome://extensions");
        println!("  2. Enable \"Developer mode\" (top right)");
        println!("  3. Click \"Load unpacked\"");
        println!("  4. Select: {}", ext_dir.display());
        println!();
        println!("The extension persists across Chrome restarts.");

        Ok(())
    }

    fn uninstall(&self) -> anyhow::Result<()> {
        manifest::remove(HOST_NAME, &["chrome"], Scope::User)?;

        // Remove the unpacked extension directory (best-effort).
        if let Ok(ext_dir) = Self::extension_dir() {
            let _ = std::fs::remove_dir_all(&ext_dir);
        }

        println!("Removed native messaging host manifest for Chrome.");
        println!();
        println!("To complete removal, also remove the extension from Chrome:");
        println!("  chrome://extensions > Attend Browser Bridge > Remove");

        Ok(())
    }
}
