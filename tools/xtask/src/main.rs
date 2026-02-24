//! Code generation tasks for the attend crate.
//!
//! Usage: `cargo xtask gen-gfm-languages`

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

    // Resolve output path relative to the workspace root.
    // The xtask binary lives at tools/xtask/, so ../../ gets us to the root.
    let manifest_dir =
        std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| "tools/xtask".to_string());
    let workspace_root = std::path::Path::new(&manifest_dir)
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(std::path::Path::new("."));
    let output = workspace_root.join(OUTPUT_PATH);

    std::fs::write(&output, &source)
        .with_context(|| format!("failed to write {}", output.display()))?;
    eprintln!("wrote {}", output.display());

    Ok(())
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match args.first().map(|s| s.as_str()) {
        Some("gen-gfm-languages") => gen_gfm_languages(),
        Some(other) => bail!("unknown command: {other}"),
        None => bail!(
            "usage: cargo xtask <command>\n\ncommands:\n  gen-gfm-languages  Regenerate src/view/gfm_languages.rs from Linguist"
        ),
    }
}
