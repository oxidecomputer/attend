//! Language detection for editor-captured file regions.
//!
//! Uses [`hyperpolyglot`] to detect the programming language from a file path,
//! then normalizes the result to a GFM-compatible fence-tag identifier. The
//! [`LanguageCache`] avoids repeated detection for the same file across
//! multiple capture cycles within a narration session.

use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};

include!("gfm_languages.rs");

/// Map a hyperpolyglot language name to a GFM fence-tag identifier.
///
/// Returns `None` when the language has no GFM syntax highlighting (not in
/// [`GFM_LANGUAGES`]).
fn normalize_language(name: &str) -> Option<String> {
    // Override table for names that don't round-trip through the default
    // lowercase + spaces→hyphens rule.
    let tag = match name {
        "C++" => "cpp",
        "C#" => "csharp",
        "F#" => "fsharp",
        "F*" => "fstar",
        "Objective-C" => "objective-c",
        "Objective-C++" => "objective-c++",
        "Objective-J" => "objective-j",
        "Batchfile" => "batchfile",
        "Emacs Lisp" => "emacs-lisp",
        "Vim Script" => "vim-script",
        "Visual Basic .NET" => "visual-basic-.net",
        _ => {
            let normalized = name.to_lowercase().replace(' ', "-");
            return if GFM_LANGUAGES.binary_search(&normalized.as_str()).is_ok() {
                Some(normalized)
            } else {
                None
            };
        }
    };
    // Validate overrides against the allowlist too.
    if GFM_LANGUAGES.binary_search(&tag).is_ok() {
        Some(tag.to_string())
    } else {
        None
    }
}

/// Detect a file's programming language from its path.
///
/// Returns a GFM-compatible fence-tag identifier, or `None` when detection
/// fails or the language lacks GFM syntax highlighting.
fn detect_language(path: &Utf8Path) -> Option<String> {
    let detection = hyperpolyglot::detect(path.as_std_path()).ok()??;
    normalize_language(detection.language())
}

/// Per-session cache of detected languages, keyed by absolute file path.
///
/// Detection runs on the whole file (via its path) regardless of which region
/// is selected. The cache is created once per narration session so repeated
/// captures of the same file don't re-run hyperpolyglot.
pub struct LanguageCache {
    cache: HashMap<Utf8PathBuf, Option<String>>,
}

impl LanguageCache {
    pub fn new() -> Self {
        Self {
            cache: HashMap::new(),
        }
    }

    /// Detect language for `path`, returning a cached result if available.
    pub fn detect(&mut self, path: &Utf8Path) -> Option<String> {
        self.cache
            .entry(path.to_path_buf())
            .or_insert_with(|| detect_language(path))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use proptest::prelude::*;

    // ── Exhaustive tests on the finite language set ──────────────────────

    /// Every entry in GFM_LANGUAGES is sorted (binary search correctness).
    #[test]
    fn gfm_languages_sorted() {
        assert!(
            GFM_LANGUAGES.windows(2).all(|w| w[0] < w[1]),
            "GFM_LANGUAGES is not strictly sorted"
        );
    }

    /// Every entry in GFM_LANGUAGES matches the fence-tag character set.
    ///
    /// GFM info strings are parsed as the first word (split on whitespace),
    /// so tags must not contain spaces. We allow lowercase ASCII, digits,
    /// hyphens, dots, `+`, and `#` — the characters that appear in real
    /// Linguist identifiers.
    #[test]
    fn gfm_languages_valid_charset() {
        for &tag in GFM_LANGUAGES {
            assert!(!tag.is_empty(), "empty tag in GFM_LANGUAGES");
            for ch in tag.chars() {
                assert!(
                    ch.is_ascii_lowercase()
                        || ch.is_ascii_digit()
                        || matches!(
                            ch,
                            '-' | '.' | '+' | '#' | '*' | '\'' | '(' | ')' | '/' | '_'
                        ),
                    "invalid character {ch:?} in GFM tag {tag:?}"
                );
            }
        }
    }

    /// Every override in normalize_language produces a valid GFM tag.
    #[test]
    fn overrides_produce_valid_tags() {
        let overrides = [
            "C++",
            "C#",
            "F#",
            "F*",
            "Objective-C",
            "Objective-C++",
            "Objective-J",
            "Batchfile",
            "Emacs Lisp",
            "Vim Script",
            "Visual Basic .NET",
        ];
        for name in &overrides {
            let result = normalize_language(name);
            assert!(
                result.is_some(),
                "override {name:?} should produce a valid GFM tag, got None"
            );
            let tag = result.unwrap();
            assert!(
                GFM_LANGUAGES.binary_search(&tag.as_str()).is_ok(),
                "override {name:?} produced {tag:?} which is not in GFM_LANGUAGES"
            );
        }
    }

    // ── Property-based tests ─────────────────────────────────────────────

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(500))]

        /// normalize_language always returns None or a GFM member.
        #[test]
        fn normalize_returns_none_or_gfm_member(s in "\\PC{0,50}") {
            if let Some(ref tag) = normalize_language(&s) {
                prop_assert!(
                    GFM_LANGUAGES.binary_search(&tag.as_str()).is_ok(),
                    "normalize({:?}) = {:?} not in GFM_LANGUAGES", s, tag
                );
            }
        }

        /// normalize_language is idempotent on the output value:
        /// if normalize(s) = Some(tag), then normalize(tag) is either
        /// Some(tag) or None — never a different tag.
        #[test]
        fn normalize_idempotent(s in "\\PC{0,50}") {
            if let Some(ref tag) = normalize_language(&s) {
                if let Some(ref tag2) = normalize_language(tag) {
                    prop_assert_eq!(
                        tag, tag2,
                        "normalize is not idempotent: {:?} -> {:?} -> {:?}", s, tag, tag2
                    );
                }
            }
        }

        /// normalize_language output contains only valid fence-tag characters.
        #[test]
        fn normalize_output_charset(s in "\\PC{0,50}") {
            if let Some(ref tag) = normalize_language(&s) {
                for ch in tag.chars() {
                    prop_assert!(
                        ch.is_ascii_lowercase()
                            || ch.is_ascii_digit()
                            || matches!(ch, '-' | '.' | '+' | '#' | '*' | '\''),
                        "invalid char {:?} in normalized tag {:?}", ch, tag
                    );
                }
            }
        }
    }

    // ── Integration tests with tempfiles ─────────────────────────────────

    /// .rs → Some("rust")
    #[test]
    fn detect_rust_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let utf8 = Utf8Path::from_path(&path).unwrap();
        assert_eq!(detect_language(utf8), Some("rust".to_string()));
    }

    /// .py → Some("python")
    #[test]
    fn detect_python_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.py");
        std::fs::write(&path, "print('hello')\n").unwrap();
        let utf8 = Utf8Path::from_path(&path).unwrap();
        assert_eq!(detect_language(utf8), Some("python".to_string()));
    }

    /// .c → Some("c")
    #[test]
    fn detect_c_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.c");
        std::fs::write(&path, "int main() { return 0; }\n").unwrap();
        let utf8 = Utf8Path::from_path(&path).unwrap();
        assert_eq!(detect_language(utf8), Some("c".to_string()));
    }

    /// .js → Some("javascript")
    #[test]
    fn detect_javascript_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.js");
        std::fs::write(&path, "console.log('hello');\n").unwrap();
        let utf8 = Utf8Path::from_path(&path).unwrap();
        assert_eq!(detect_language(utf8), Some("javascript".to_string()));
    }

    /// Unknown extension → None
    #[test]
    fn detect_unknown_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.xyzzy123");
        std::fs::write(&path, "unknown content\n").unwrap();
        let utf8 = Utf8Path::from_path(&path).unwrap();
        assert_eq!(detect_language(utf8), None);
    }

    /// Nonexistent path → None
    #[test]
    fn detect_nonexistent_path() {
        let utf8 = Utf8Path::new("/nonexistent/path/to/file.rs");
        assert_eq!(detect_language(utf8), None);
    }

    /// LanguageCache returns cached results and deduplicates detection calls.
    #[test]
    fn cache_hit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cached.rs");
        std::fs::write(&path, "fn cached() {}\n").unwrap();
        let utf8 = Utf8Path::from_path(&path).unwrap();

        let mut cache = LanguageCache::new();
        let first = cache.detect(utf8);
        let second = cache.detect(utf8);
        assert_eq!(first, second);
        assert_eq!(first, Some("rust".to_string()));
        assert_eq!(cache.cache.len(), 1);
    }

    // ── GFM_LANGUAGES sync check (ignored: requires network) ────────────

    /// Verify the checked-in GFM_LANGUAGES is in sync with Linguist.
    ///
    /// This test fetches `languages.yml` from GitHub and compares against
    /// the checked-in allowlist. Run with:
    ///
    ///     cargo test -- --ignored gfm_allowlist_in_sync
    ///
    /// If it fails, regenerate with `cargo xtask gen-gfm-languages`.
    #[test]
    #[ignore]
    fn gfm_allowlist_in_sync() {
        use std::collections::BTreeSet;

        #[derive(serde::Deserialize)]
        struct Lang {
            #[serde(default)]
            tm_scope: Option<String>,
            #[serde(default)]
            aliases: Vec<String>,
        }

        let url = "https://raw.githubusercontent.com/github-linguist/linguist/master/lib/linguist/languages.yml";
        let body: String = ureq::get(url)
            .call()
            .expect("failed to fetch languages.yml")
            .body_mut()
            .read_to_string()
            .expect("failed to read body");
        let languages: std::collections::BTreeMap<String, Lang> =
            serde_yaml::from_str(&body).expect("failed to parse YAML");

        let mut expected = BTreeSet::new();
        for (name, lang) in &languages {
            if lang.tm_scope.as_deref() == Some("none") {
                continue;
            }
            expected.insert(name.to_lowercase().replace(' ', "-"));
            for alias in &lang.aliases {
                expected.insert(alias.to_lowercase().replace(' ', "-"));
            }
        }

        let checked_in: BTreeSet<&str> = GFM_LANGUAGES.iter().copied().collect();
        let expected_refs: BTreeSet<&str> = expected.iter().map(|s| s.as_str()).collect();

        let missing: Vec<_> = expected_refs.difference(&checked_in).collect();
        let extra: Vec<_> = checked_in.difference(&expected_refs).collect();

        assert!(
            missing.is_empty() && extra.is_empty(),
            "GFM_LANGUAGES out of sync with Linguist.\n  missing: {missing:?}\n  extra: {extra:?}\n\
             Regenerate with: cargo xtask gen-gfm-languages"
        );
    }
}
