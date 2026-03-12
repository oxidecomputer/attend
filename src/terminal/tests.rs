use super::*;

/// A line whose visible length is strictly shorter than max_cols should
/// pass through completely unchanged: no truncation, no added escapes.
#[test]
fn short_line_unchanged() {
    assert_eq!(truncate_line("hi", 10), "hi");
}

/// A line whose visible length equals max_cols must not be truncated.
/// A line one visible character over max_cols must be truncated and end
/// with RESET + "…".
#[test]
fn truncation_at_boundary() {
    // Exactly at max_cols: not truncated.
    assert_eq!(truncate_line("hello", 5), "hello");
    // One over: truncated.
    assert_eq!(truncate_line("hello!", 5), "hell\x1b[0m…");
}

/// ANSI escape sequences do not contribute to visible width. A string
/// whose visible content fits within max_cols should not be truncated
/// even if the byte length far exceeds it.
#[test]
fn ansi_escape_passthrough() {
    let styled = "\x1b[1mhello\x1b[0m";
    // 5 visible chars, max_cols = 5: should not truncate.
    assert_eq!(truncate_line(styled, 5), styled);
}

/// When truncation occurs inside ANSI-styled text, the output must
/// include a RESET (\x1b[0m) before the "…" to avoid style leaking.
#[test]
fn ansi_mid_truncation() {
    let styled = "\x1b[1mhello world\x1b[0m";
    // 11 visible chars, max_cols = 8: truncate after 7 visible chars.
    let result = truncate_line(styled, 8);
    assert!(result.ends_with("\x1b[0m…"), "result was: {result:?}");
    // The visible portion before the ellipsis should be "hello w" (7 chars).
    assert!(
        result.starts_with("\x1b[1mhello w"),
        "result was: {result:?}"
    );
}

/// Multi-byte UTF-8 characters each count as one visible character.
/// Truncation must not split a multi-byte character.
#[test]
fn utf8_multi_byte() {
    // "café" is 4 visible characters ('c', 'a', 'f', 'é').
    assert_eq!(truncate_line("café", 4), "café");
    // max_cols = 4 with 5-char input: truncate after 3 visible chars.
    assert_eq!(truncate_line("café!", 4), "caf\x1b[0m…");
}

/// max_cols of zero always produces the empty string.
#[test]
fn zero_max_cols() {
    assert_eq!(truncate_line("anything", 0), "");
}

/// An empty input line always produces the empty string.
#[test]
fn empty_line() {
    assert_eq!(truncate_line("", 5), "");
}
