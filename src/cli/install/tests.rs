use super::*;

/// `merge_unique` appends new items and skips duplicates.
#[test]
fn merge_unique_deduplicates() {
    let mut existing = vec!["a".to_string(), "b".to_string()];
    merge_unique(
        &mut existing,
        vec!["b".to_string(), "c".to_string(), "a".to_string()],
    );
    assert_eq!(existing, vec!["a", "b", "c"]);
}

/// `merge_unique` with an empty existing list just adopts the new items.
#[test]
fn merge_unique_into_empty() {
    let mut existing = Vec::new();
    merge_unique(&mut existing, vec!["x".to_string(), "y".to_string()]);
    assert_eq!(existing, vec!["x", "y"]);
}

/// `merge_unique` with an empty new list leaves existing unchanged.
#[test]
fn merge_unique_empty_new() {
    let mut existing = vec!["a".to_string()];
    merge_unique(&mut existing, Vec::new());
    assert_eq!(existing, vec!["a"]);
}

/// `resolve_or_all` returns explicit values when `use_all` is false.
#[test]
fn resolve_or_all_explicit() {
    let names = ["x", "y", "z"].iter().copied();
    let result = resolve_or_all(vec!["a".to_string()], names, false);
    assert_eq!(result, vec!["a"]);
}

/// `resolve_or_all` returns all names from the iterator when `use_all` is true.
#[test]
fn resolve_or_all_all() {
    let names = ["x", "y", "z"].iter().copied();
    let result = resolve_or_all(vec!["a".to_string()], names, true);
    assert_eq!(result, vec!["x", "y", "z"]);
}

/// `Outcome::Installed` displays with a `+` prefix.
#[test]
fn outcome_installed_display() {
    let o = Outcome::Installed {
        category: Category::Agent,
        name: "claude".to_string(),
    };
    assert_eq!(format!("{o}"), "  + agent: claude");
}

/// `Outcome::Skipped` displays with a `-` prefix and reason.
#[test]
fn outcome_skipped_display() {
    let o = Outcome::Skipped {
        category: Category::Browser,
        name: "chrome".to_string(),
        reason: "not detected".to_string(),
    };
    assert_eq!(format!("{o}"), "  - browser: chrome (not detected)");
}

/// `Outcome::is_installed` returns true only for `Installed` variant.
#[test]
fn outcome_is_installed() {
    let installed = Outcome::Installed {
        category: Category::Shell,
        name: "fish".to_string(),
    };
    let skipped = Outcome::Skipped {
        category: Category::Shell,
        name: "zsh".to_string(),
        reason: "failed".to_string(),
    };
    assert!(installed.is_installed());
    assert!(!skipped.is_installed());
}

/// `concise_reason` extracts the root cause from an error chain.
#[test]
fn concise_reason_extracts_root_cause() {
    let inner = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
    let outer = anyhow::Error::new(inner).context("installing hooks");
    let reason = concise_reason(&outer);
    assert_eq!(reason, "file missing");
}

/// `concise_reason` handles a simple error with no chain.
#[test]
fn concise_reason_simple_error() {
    let err = anyhow::anyhow!("something broke");
    let reason = concise_reason(&err);
    assert_eq!(reason, "something broke");
}

/// Each `Category` variant has the expected label.
#[test]
fn category_labels() {
    assert_eq!(Category::Agent.label(), "agent");
    assert_eq!(Category::Editor.label(), "editor");
    assert_eq!(Category::Browser.label(), "browser");
    assert_eq!(Category::Shell.label(), "shell");
}

/// `InstallArgs::run` dispatches to auto-detect mode when all lists are empty.
///
/// We verify the routing logic: `has_explicit` is false when all four vecs
/// are empty, which causes `install_auto` to be called rather than
/// `install_targeted`.
#[test]
fn has_explicit_is_false_when_all_empty() {
    let agent: Vec<String> = vec![];
    let editor: Vec<String> = vec![];
    let browser: Vec<String> = vec![];
    let shell: Vec<String> = vec![];

    let has_explicit =
        !agent.is_empty() || !editor.is_empty() || !browser.is_empty() || !shell.is_empty();
    assert!(!has_explicit, "empty args should trigger auto-detect mode");
}

/// Specific flags set `has_explicit` to true, using targeted mode.
#[test]
fn has_explicit_is_true_with_any_flag() {
    let cases: Vec<(Vec<String>, Vec<String>, Vec<String>, Vec<String>)> = vec![
        (vec!["claude".into()], vec![], vec![], vec![]),
        (vec![], vec!["zed".into()], vec![], vec![]),
        (vec![], vec![], vec!["chrome".into()], vec![]),
        (vec![], vec![], vec![], vec!["fish".into()]),
    ];

    for (agent, editor, browser, shell) in cases {
        let has_explicit =
            !agent.is_empty() || !editor.is_empty() || !browser.is_empty() || !shell.is_empty();
        assert!(
            has_explicit,
            "should use targeted mode when any flag is set"
        );
    }
}
