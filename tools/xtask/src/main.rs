//! Code generation and release tasks for the attend crate.
//!
//! Usage:
//!   cargo xtask gen-gfm-languages    Regenerate GFM language list
//!   cargo xtask sign-extension       Sign the Firefox extension via AMO

use std::collections::BTreeSet;

use anyhow::{Context, bail};
use serde::Deserialize;

/// Partial schema for a Linguist `languages.yml` entry.
#[derive(Deserialize)]
struct Language {
    #[serde(default)]
    tm_scope: Option<String>,
    #[serde(default)]
    aliases: Vec<String>,
}

const LINGUIST_URL: &str =
    "https://raw.githubusercontent.com/github-linguist/linguist/master/lib/linguist/languages.yml";

/// Destination path for the generated file (relative to the workspace root).
const OUTPUT_PATH: &str = "src/view/gfm_languages.rs";

/// Normalize a language name to a GFM fence-tag identifier:
/// lowercase, spaces→hyphens.
fn normalize(name: &str) -> String {
    name.to_lowercase().replace(' ', "-")
}

fn fetch_languages() -> anyhow::Result<BTreeSet<String>> {
    eprintln!("fetching {LINGUIST_URL}");
    let body: String = ureq::get(LINGUIST_URL)
        .call()?
        .body_mut()
        .read_to_string()?;
    let languages: std::collections::BTreeMap<String, Language> =
        serde_yaml::from_str(&body).context("failed to parse languages.yml")?;

    let mut tags = BTreeSet::new();

    for (name, lang) in &languages {
        // Skip languages without a TextMate grammar: GFM can't highlight them.
        if lang.tm_scope.as_deref() == Some("none") {
            continue;
        }

        tags.insert(normalize(name));
        for alias in &lang.aliases {
            tags.insert(normalize(alias));
        }
    }

    Ok(tags)
}

fn generate_source(tags: &BTreeSet<String>) -> String {
    let mut out = String::new();
    out.push_str(
        "\
/// GFM-compatible language identifiers for fenced code blocks.
///
/// Generated from GitHub Linguist's `languages.yml`. Regenerate with:
///
///     cargo xtask gen-gfm-languages
///
/// Sorted for binary search. Only languages with a TextMate grammar
/// (`tm_scope != \"none\"`) are included: canonical names and aliases,
/// all lowercased.
pub const GFM_LANGUAGES: &[&str] = &[\n",
    );
    for tag in tags {
        out.push_str(&format!("    {tag:?},\n"));
    }
    out.push_str("];\n");
    out
}

fn gen_gfm_languages() -> anyhow::Result<()> {
    let tags = fetch_languages()?;
    eprintln!("{} language tags collected", tags.len());

    let source = generate_source(&tags);

    let output = workspace_root().join(OUTPUT_PATH);

    std::fs::write(&output, &source)
        .with_context(|| format!("failed to write {}", output.display()))?;
    eprintln!("wrote {}", output.display());

    Ok(())
}

/// Resolve the workspace root from CARGO_MANIFEST_DIR (tools/xtask → ../..).
fn workspace_root() -> std::path::PathBuf {
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| "tools/xtask".to_string());
    std::path::Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new("."))
        .to_path_buf()
}

/// Sign the Firefox extension as an unlisted AMO add-on.
///
/// Requires `web-ext` on PATH and two environment variables:
///   AMO_JWT_ISSUER  — API key (JWT issuer) from addons.mozilla.org
///   AMO_JWT_SECRET  — API secret from addons.mozilla.org
///
/// Produces `extension/attend.xpi` in the workspace root. Rebuild attend
/// after signing to embed the .xpi in the binary.
fn sign_extension() -> anyhow::Result<()> {
    // Check prerequisites.
    which::which("web-ext")
        .context("web-ext not found on PATH (install with: npm install -g web-ext)")?;
    let api_key =
        std::env::var("AMO_JWT_ISSUER").context("AMO_JWT_ISSUER environment variable not set")?;
    let api_secret =
        std::env::var("AMO_JWT_SECRET").context("AMO_JWT_SECRET environment variable not set")?;

    let root = workspace_root();
    let ext_dir = root.join("extension");
    let artifacts = tempfile::tempdir().context("failed to create temp directory")?;

    // Assemble a clean source directory with only the Firefox files.
    // web-ext picks up manifest.json automatically; we must exclude Chrome's.
    let source = tempfile::tempdir().context("failed to create temp directory")?;
    for name in ["manifest.json", "content.js", "background.js"] {
        std::fs::copy(ext_dir.join(name), source.path().join(name))
            .with_context(|| format!("failed to copy {name}"))?;
    }

    eprintln!("Signing extension via AMO (unlisted channel)...");
    let status = std::process::Command::new("web-ext")
        .args(["sign", "--channel=unlisted", "--source-dir"])
        .arg(source.path())
        .arg("--artifacts-dir")
        .arg(artifacts.path())
        .args(["--api-key", &api_key, "--api-secret", &api_secret])
        .status()
        .context("failed to run web-ext")?;

    if !status.success() {
        bail!("web-ext sign failed (exit code: {status})");
    }

    // Find the produced .xpi in the artifacts directory.
    let xpi = std::fs::read_dir(artifacts.path())?
        .filter_map(|e| e.ok())
        .find(|e| e.path().extension().is_some_and(|ext| ext == "xpi"))
        .ok_or_else(|| anyhow::anyhow!("no .xpi found in web-ext artifacts"))?;

    let dest = root.join("extension").join("attend.xpi");
    std::fs::copy(xpi.path(), &dest)
        .with_context(|| format!("failed to copy .xpi to {}", dest.display()))?;

    eprintln!("signed .xpi written to: {}", dest.display());
    eprintln!("rebuild attend to embed it: cargo build --release");

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(|s| s.as_str()) {
        Some("gen-gfm-languages") => gen_gfm_languages(),
        Some("sign-extension") => sign_extension(),
        Some(other) => bail!("unknown command: {other}"),
        None => bail!(
            "usage: cargo xtask <command>\n\n\
             commands:\n  \
             gen-gfm-languages  Regenerate src/view/gfm_languages.rs from Linguist\n  \
             sign-extension     Sign the Firefox extension via AMO (unlisted)"
        ),
    }
}
