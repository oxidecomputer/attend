//! Markdown rendering for merged narration events.
//!
//! Takes a sorted/compressed event list and produces a markdown document
//! interleaving prose (from speech) with fenced code blocks (from editor
//! navigation) and fenced diff blocks (from file changes).

use serde::{Deserialize, Serialize};

use super::merge::{self, Event};

/// Controls collapsing of large code snippets and diffs.
///
/// When a fenced block exceeds `threshold` lines, only the first `head`
/// and last `tail` lines are kept, with a `// ... (N lines omitted)` marker
/// in between.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SnipConfig {
    /// Blocks with more lines than this are snipped.
    pub threshold: usize,
    /// Lines to keep at the start of a snipped block.
    pub head: usize,
    /// Lines to keep at the end of a snipped block.
    pub tail: usize,
}

impl Default for SnipConfig {
    fn default() -> Self {
        Self {
            threshold: 5,
            head: 3,
            tail: 2,
        }
    }
}

/// Collapse a multi-line string if it exceeds the snip threshold.
///
/// Returns the original string unchanged if it fits, otherwise keeps the
/// first `head` and last `tail` lines with an omission marker.
///
/// When `first_line` is provided, the marker includes the actual line range
/// (e.g. `lines 45-78`) so an agent can request exactly those lines.
fn snip(text: &str, cfg: SnipConfig, first_line: Option<usize>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= cfg.threshold || cfg.head + cfg.tail >= lines.len() {
        return text.to_string();
    }
    let omitted = lines.len() - cfg.head - cfg.tail;
    let mut out = String::new();
    for line in &lines[..cfg.head] {
        out.push_str(line);
        out.push('\n');
    }
    match first_line {
        Some(base) => {
            let start = base + cfg.head;
            let end = start + omitted - 1;
            out.push_str(&format!("// ... (lines {start}-{end} omitted)\n"));
        }
        None => {
            out.push_str(&format!("// ... ({omitted} lines omitted)\n"));
        }
    }
    for line in &lines[lines.len() - cfg.tail..] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Returns true if `text` starts with a character that should attach to
/// the preceding word without a space (punctuation, contractions).
fn starts_with_punctuation(text: &str) -> bool {
    text.as_bytes().first().is_some_and(|&b| {
        matches!(
            b,
            b'.' | b',' | b';' | b':' | b'!' | b'?' | b'\'' | b'"' | b')' | b']' | b'}' | b'%'
        )
    })
}

/// Returns true if a Whisper segment is a noise/hallucination marker
/// like `[typing sounds]`, `[music]`, `(buzzing)`, etc.
fn is_noise_marker(text: &str) -> bool {
    let t = text.trim();
    (t.starts_with('[') && t.ends_with(']')) || (t.starts_with('(') && t.ends_with(')'))
}

/// Clean up Whisper transcription artifacts within a single segment.
///
/// Handles intra-segment spacing (`I 'm` → `I'm`) and strips
/// bracketed noise markers (`[typing sounds]`).
fn clean_whisper_text(raw: &str) -> String {
    let chars: Vec<char> = raw.chars().collect();
    let mut out = String::with_capacity(raw.len());

    for (i, &ch) in chars.iter().enumerate() {
        if ch == ' ' {
            // Look ahead past any further spaces to find the first non-space.
            let next_non_space = chars[i + 1..].iter().find(|&&c| c != ' ');
            if let Some(&nch) = next_non_space
                && matches!(
                    nch,
                    '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | ')' | ']' | '}' | '%'
                )
            {
                continue;
            }
        }
        out.push(ch);
    }

    out
}

/// Render a sorted/compressed event list as markdown.
///
/// The output interleaves prose text with fenced code/diff blocks:
/// - Words become flowing prose text
/// - Editor snapshots become fenced code blocks with `// path:line` headers
/// - File diffs become fenced diff blocks with `// path` headers
pub fn render_markdown(events: &[Event], snip_cfg: SnipConfig) -> String {
    let mut out = String::new();
    let mut in_prose = false;

    for event in events.iter() {
        match event {
            Event::Words { text, .. } => {
                let cleaned = clean_whisper_text(text);
                if cleaned.is_empty() || is_noise_marker(&cleaned) {
                    continue;
                }
                if !in_prose && !out.is_empty() {
                    out.push('\n');
                }
                // Skip the space before punctuation that attaches to previous word
                if in_prose && !starts_with_punctuation(&cleaned) {
                    out.push(' ');
                }
                out.push_str(&cleaned);
                in_prose = true;
            }
            Event::EditorSnapshot { regions, .. } => {
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                for region in regions {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                    let annotated = crate::view::apply_markers(
                        &region.content,
                        region.first_line,
                        &region.selections,
                    );
                    let snipped = snip(&annotated, snip_cfg, Some(region.first_line as usize));
                    out.push_str("```\n");
                    out.push_str(&format!("// {}:{}\n", region.path, region.first_line));
                    out.push_str(&snipped);
                    if !snipped.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n");
                }
            }
            Event::FileDiff { path, old, new, .. } => {
                if old == new {
                    continue;
                }
                let diff = merge::unified_diff(old, new);
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                out.push_str("```diff\n");
                out.push_str(&format!("// {path}\n"));
                // Diffs are not snipped: they represent transient on-disk state
                // that cannot be reconstructed after the fact.
                out.push_str(&diff);
                if !diff.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n");
            }
            Event::ExternalSelection {
                app,
                window_title,
                text,
                ..
            } => {
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                // Render as a blockquote with source annotation.
                // External selections are not snipped: they represent ephemeral
                // state (accessibility API) that cannot be reconstructed.
                let source = if window_title.is_empty() {
                    app.to_string()
                } else {
                    format!("{app}: {window_title}")
                };
                out.push_str(&format!("> [{source}] \"{}\"\n", text.trim()));
            }
            Event::BrowserSelection {
                url,
                title,
                text,
                is_code,
                ..
            } => {
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                // Browser selections are not snipped: they represent ephemeral
                // page content that cannot be reconstructed after navigation.
                let link = if title.is_empty() {
                    format!("> <{url}>")
                } else {
                    format!("> [{title}]({url})")
                };
                if *is_code {
                    out.push_str(&link);
                    out.push('\n');
                    out.push_str("```\n");
                    out.push_str(text);
                    if !text.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n");
                } else {
                    out.push_str(&link);
                    out.push('\n');
                    out.push_str(&format!("> \"{}\"\n", text.trim()));
                }
            }
        }
    }

    if in_prose {
        out.push('\n');
    }

    out
}

/// Merge all events chronologically and format as markdown.
///
/// Convenience function that calls [`merge::compress_and_merge`] followed by
/// [`render_markdown`]. Used primarily in tests.
#[cfg(test)]
pub fn format_markdown(events: &mut Vec<Event>, snip_cfg: SnipConfig) -> String {
    merge::compress_and_merge(events);
    render_markdown(events, snip_cfg)
}

#[cfg(test)]
mod tests {
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
}
