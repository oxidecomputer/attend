use super::*;

use proptest::prelude::*;

// -- clean_whisper_text --

/// Space before period is removed.
#[test]
fn clean_whisper_space_before_period() {
    assert_eq!(clean_whisper_text("test ."), "test.");
}

/// Space before apostrophe in contraction is removed.
#[test]
fn clean_whisper_contraction() {
    assert_eq!(clean_whisper_text("I 'm going"), "I'm going");
}

/// Space before comma is removed.
#[test]
fn clean_whisper_comma() {
    assert_eq!(clean_whisper_text("Now , let"), "Now, let");
}

/// Multiple Whisper artifacts in one string are all cleaned.
#[test]
fn clean_whisper_multiple() {
    assert_eq!(
        clean_whisper_text("Hello , I 'm here . Great !"),
        "Hello, I'm here. Great!"
    );
}

/// Text without Whisper artifacts passes through unchanged.
#[test]
fn clean_whisper_no_change() {
    assert_eq!(clean_whisper_text("no changes here"), "no changes here");
}

/// Normal spaces between words are preserved.
#[test]
fn clean_whisper_preserves_spaces() {
    assert_eq!(clean_whisper_text("a b c"), "a b c");
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// clean_whisper_text is idempotent: cleaning twice equals cleaning once.
    #[test]
    fn clean_whisper_idempotent(input in "[ -~]{0,100}") {
        let once = clean_whisper_text(&input);
        let twice = clean_whisper_text(&once);
        prop_assert_eq!(&once, &twice);
    }

    /// clean_whisper_text never increases string length.
    #[test]
    fn clean_whisper_never_grows(input in "[ -~]{0,100}") {
        let cleaned = clean_whisper_text(&input);
        prop_assert!(
            cleaned.len() <= input.len(),
            "cleaned ({}) longer than input ({})",
            cleaned.len(),
            input.len()
        );
    }

    /// clean_whisper_text preserves all non-space characters in order.
    #[test]
    fn clean_whisper_preserves_non_space(input in "[ -~]{0,100}") {
        let cleaned = clean_whisper_text(&input);
        let input_non_space: String = input.chars().filter(|&c| c != ' ').collect();
        let cleaned_non_space: String = cleaned.chars().filter(|&c| c != ' ').collect();
        prop_assert_eq!(input_non_space, cleaned_non_space);
    }
}

// -- is_noise_marker --

/// Bracketed and parenthesized markers are recognized; plain text is not.
#[test]
fn noise_marker_parenthesized() {
    assert!(is_noise_marker("[music]"));
    assert!(is_noise_marker("(buzzing)"));
    assert!(is_noise_marker("  [typing sounds]  "));
    assert!(!is_noise_marker("hello"));
    assert!(!is_noise_marker("[not closed"));
}

// -- snip --

/// Text at or below the snip threshold passes through unchanged.
#[test]
fn snip_below_threshold_unchanged() {
    let text = "line1\nline2\nline3\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    assert_eq!(snip(text, cfg, None), text);
}

/// Text above the threshold keeps head/tail lines with an omission count.
#[test]
fn snip_above_threshold_collapses() {
    let text = "a\nb\nc\nd\ne\nf\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    // Without line numbers
    assert_eq!(snip(text, cfg, None), "a\nb\n// ... (3 lines omitted)\nf\n");
}

/// Snip marker includes actual line numbers when first_line is provided.
#[test]
fn snip_with_line_range() {
    let text = "a\nb\nc\nd\ne\nf\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    // first_line=10: head keeps lines 10-11, omits 12-14, tail keeps line 15
    assert_eq!(
        snip(text, cfg, Some(10)),
        "a\nb\n// ... (lines 12-14 omitted)\nf\n"
    );
}

/// Text at exactly the threshold passes through unchanged.
#[test]
fn snip_at_exact_threshold_unchanged() {
    let text = (1..=5)
        .map(|i| format!("line{i}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    assert_eq!(snip(&text, cfg, Some(1)), text);
}

/// When head + tail >= total lines, snip should not panic or produce
/// malformed output (overlapping head/tail).
#[test]
fn snip_head_tail_overlap() {
    // 6 lines with head=4, tail=4 → head+tail=8 > 6 lines
    let text = "a\nb\nc\nd\ne\nf\n";
    let cfg = SnipConfig {
        threshold: 3, // trigger snipping (6 > 3)
        head: 4,
        tail: 4,
    };
    let result = snip(text, cfg, None);
    // With overlapping head/tail, all original lines should survive
    // (nothing to omit). Verify no panic and no duplication.
    let result_lines: Vec<&str> = result.lines().collect();
    let input_lines: Vec<&str> = text.lines().collect();
    // Every input line should appear at least once.
    for line in &input_lines {
        assert!(
            result_lines.contains(line),
            "line {:?} missing from snip output: {:?}",
            line,
            result
        );
    }
}

// -- redacted marker rendering --

/// Single Redacted event renders with ✂ prefix and no count.
#[test]
fn render_redacted_singular() {
    use super::super::merge::RedactedKind;

    let events = vec![Event::Redacted {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        kind: RedactedKind::EditorSnapshot,
        keys: vec!["a.rs".to_string()],
    }];
    let md = render_markdown(&events, SnipConfig::default());
    assert_eq!(md.trim(), "\u{2702} file");
}

/// Multiple-key Redacted event renders with count.
#[test]
fn render_redacted_plural() {
    use super::super::merge::RedactedKind;

    let events = vec![Event::Redacted {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        kind: RedactedKind::FileDiff,
        keys: vec!["a.rs".to_string(), "b.rs".to_string(), "c.rs".to_string()],
    }];
    let md = render_markdown(&events, SnipConfig::default());
    assert_eq!(md.trim(), "\u{2702} 3 edits");
}

/// ShellCommand redaction uses "command"/"commands" label.
#[test]
fn render_redacted_shell_command() {
    use super::super::merge::RedactedKind;

    let events = vec![Event::Redacted {
        timestamp: chrono::DateTime::UNIX_EPOCH,
        kind: RedactedKind::ShellCommand,
        keys: vec!["ls".to_string(), "pwd".to_string()],
    }];
    let md = render_markdown(&events, SnipConfig::default());
    assert_eq!(md.trim(), "\u{2702} 2 commands");
}

/// Adjacent Redacted events of different kinds render comma-separated.
#[test]
fn render_redacted_comma_separated() {
    use super::super::merge::RedactedKind;

    let events = vec![
        Event::Redacted {
            timestamp: chrono::DateTime::UNIX_EPOCH,
            kind: RedactedKind::EditorSnapshot,
            keys: vec!["a.rs".to_string(), "b.rs".to_string()],
        },
        Event::Redacted {
            timestamp: chrono::DateTime::UNIX_EPOCH,
            kind: RedactedKind::ShellCommand,
            keys: vec!["ls".to_string()],
        },
    ];
    let md = render_markdown(&events, SnipConfig::default());
    assert_eq!(md.trim(), "\u{2702} 2 files, command");
}
