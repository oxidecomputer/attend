//! Firefox browser integration: native messaging host manifest + signed XPI.
//!
//! When a signed `.xpi` is embedded at build time (via `cargo xtask
//! sign-extension`), install writes it to disk and opens it in Firefox for
//! permanent installation. Otherwise, prints development-mode instructions.

use std::path::Path;

use native_messaging::install::{manifest, paths::Scope};

/// Extension ID for the attend Firefox add-on.
const EXTENSION_ID: &str = "attend@oxide.computer";

/// Native messaging host name (must match the extension's `sendNativeMessage` call).
const HOST_NAME: &str = "attend";

/// Human-readable description for the native messaging manifest.
const DESCRIPTION: &str = "Attend browser bridge: captures selections for narration";

/// Signed `.xpi` bytes, embedded at compile time when `extension/attend.xpi`
/// exists. Produced by `cargo xtask sign-extension`.
#[cfg(has_signed_xpi)]
fn xpi_bytes() -> Option<&'static [u8]> {
    Some(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/extension/attend.xpi"
    )))
}

#[cfg(not(has_signed_xpi))]
fn xpi_bytes() -> Option<&'static [u8]> {
    None
}

pub struct Firefox;

impl super::Browser for Firefox {
    fn name(&self) -> &'static str {
        "firefox"
    }

    fn install(&self, bin_cmd: &str) -> anyhow::Result<()> {
        let exe_path = Path::new(bin_cmd);

        manifest::install(
            HOST_NAME,
            DESCRIPTION,
            exe_path,
            &[],                         // chrome origins (unused)
            &[EXTENSION_ID.to_string()], // firefox extensions
            &["firefox"],
            Scope::User,
        )?;

        // Verify the manifest was written correctly.
        let ok = manifest::verify_installed(HOST_NAME, Some(&["firefox"]), Scope::User)?;
        if !ok {
            anyhow::bail!("native messaging manifest verification failed");
        }

        println!("Installed native messaging host manifest for Firefox.");

        if let Some(xpi) = xpi_bytes() {
            // Write the signed .xpi to a persistent location and open it.
            let data_dir = dirs::data_dir()
                .ok_or_else(|| anyhow::anyhow!("cannot determine data directory"))?;
            let xpi_dir = data_dir.join("attend");
            std::fs::create_dir_all(&xpi_dir)?;
            let xpi_path = xpi_dir.join("attend-browser-bridge.xpi");
            std::fs::write(&xpi_path, xpi)?;

            println!("Opening the extension in Firefox for installation...");
            let opened = open_xpi(&xpi_path);
            if !opened {
                println!("Could not open automatically. Install manually by opening:");
                println!("  {}", xpi_path.display());
            }
        } else {
            println!("Next, install the attend browser extension in Firefox:");
            println!();
            println!("  Development (temporary, until Firefox restarts):");
            println!("    1. Open about:debugging#/runtime/this-firefox");
            println!("    2. Click \"Load Temporary Add-on\"");
            println!("    3. Select extension/manifest.json from the attend source tree");
            println!();
            println!("  Permanent (requires signing):");
            println!("    Build a signed .xpi with: cargo xtask sign-extension");
            println!("    Then rebuild attend and re-run install.");
        }

        Ok(())
    }

    fn uninstall(&self) -> anyhow::Result<()> {
        manifest::remove(HOST_NAME, &["firefox"], Scope::User)?;

        // Clean up the .xpi file (best-effort).
        if let Some(data_dir) = dirs::data_dir() {
            let xpi_path = data_dir.join("attend").join("attend-browser-bridge.xpi");
            let _ = std::fs::remove_file(xpi_path);
        }

        println!("Removed native messaging host manifest for Firefox.");
        println!();
        println!("To complete removal, also uninstall the attend browser extension:");
        println!("  Firefox > Add-ons and Themes > Extensions > Attend Browser Bridge > Remove");

        Ok(())
    }
}

/// Try to open the `.xpi` in Firefox via the platform's default handler.
fn open_xpi(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .args(["-a", "Firefox"])
            .arg(path)
            .status()
            .is_ok_and(|s| s.success())
    }

    #[cfg(target_os = "linux")]
    {
        // xdg-open won't know .xpi → Firefox, so invoke Firefox directly.
        // Try common binary names; packaged installs may use either.
        for bin in ["firefox", "firefox-esr"] {
            if let Ok(status) = std::process::Command::new(bin).arg(path).status() {
                if status.success() {
                    return true;
                }
            }
        }
        false
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = path;
        false
    }
}
