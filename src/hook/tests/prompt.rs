use super::super::*;

// ---------------------------------------------------------------------------
// is_attend_prompt tests
// ---------------------------------------------------------------------------

/// Exact `/attend` match.
#[test]
fn exact() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("/attend".into()),
        },
        ..Default::default()
    };
    assert!(is_attend_prompt(&input));
}

/// `/attend` with surrounding whitespace.
#[test]
fn with_whitespace() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("  /attend  ".into()),
        },
        ..Default::default()
    };
    assert!(is_attend_prompt(&input));
}

/// Non-attend prompt text.
#[test]
fn different_text() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("hello world".into()),
        },
        ..Default::default()
    };
    assert!(!is_attend_prompt(&input));
}

/// No prompt field at all.
#[test]
fn no_prompt_field() {
    let input = HookInput::default();
    assert!(!is_attend_prompt(&input));
}

/// Partial match: `/attend to this` should not match.
#[test]
fn partial() {
    let input = HookInput {
        kind: HookKind::UserPrompt {
            prompt: Some("/attend to this".into()),
        },
        ..Default::default()
    };
    assert!(!is_attend_prompt(&input));
}
