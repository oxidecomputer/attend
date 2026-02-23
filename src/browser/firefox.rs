//! Firefox browser integration: native messaging host manifest management.

use std::path::Path;

use native_messaging::install::{manifest, paths::Scope};

/// Extension ID for the attend Firefox add-on.
const EXTENSION_ID: &str = "attend@oxide.computer";

/// Native messaging host name (must match the extension's `connectNative`/
/// `sendNativeMessage` call).
const HOST_NAME: &str = "attend";

/// Human-readable description for the native messaging manifest.
const DESCRIPTION: &str = "Attend browser bridge: captures selections for narration";

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
            &["firefox"],                // browser keys
            Scope::User,
        )?;

        // Verify the manifest was written correctly.
        let ok = manifest::verify_installed(HOST_NAME, Some(&["firefox"]), Scope::User)?;
        if !ok {
            anyhow::bail!("native messaging manifest verification failed");
        }

        println!("Installed native messaging host manifest for Firefox.");
        println!();
        println!("Next, install the attend browser extension in Firefox:");
        println!();
        println!("  Development (temporary, until Firefox restarts):");
        println!("    1. Open about:debugging#/runtime/this-firefox");
        println!("    2. Click \"Load Temporary Add-on\"");
        println!("    3. Select extension/manifest.json from the attend source tree");
        println!();
        println!("  Permanent (requires signing):");
        println!("    1. Install web-ext: npm install -g web-ext");
        println!("    2. cd extension && web-ext sign --api-key=... --api-secret=...");
        println!("    3. Install the generated .xpi file in Firefox");
        println!(
            "    See https://extensionworkshop.com/documentation/develop/getting-started-with-web-ext/"
        );

        Ok(())
    }

    fn uninstall(&self) -> anyhow::Result<()> {
        manifest::remove(HOST_NAME, &["firefox"], Scope::User)?;

        println!("Removed native messaging host manifest for Firefox.");
        println!();
        println!("To complete removal, also uninstall the attend browser extension:");
        println!("  Firefox > Add-ons and Themes > Extensions > Attend Browser Bridge > Remove");

        Ok(())
    }
}
