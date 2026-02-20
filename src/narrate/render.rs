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
    if lines.len() <= cfg.threshold {
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
    let mut out = String::with_capacity(raw.len());
    let mut chars = raw.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == ' '
            && let Some(&next) = chars.peek()
            && matches!(
                next,
                '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '"' | ')' | ']' | '}' | '%'
            )
        {
            continue;
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
            Event::EditorSnapshot { rendered, .. } => {
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                for file in rendered {
                    if !out.is_empty() && !out.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push('\n');
                    let snipped = snip(&file.content, snip_cfg, Some(file.first_line as usize));
                    out.push_str("```\n");
                    out.push_str(&format!("// {}:{}\n", file.path, file.first_line));
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
                let snipped = snip(&diff, snip_cfg, None);
                out.push_str("```diff\n");
                out.push_str(&format!("// {path}\n"));
                out.push_str(&snipped);
                if !snipped.ends_with('\n') {
                    out.push('\n');
                }
                out.push_str("```\n");
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

    // -- clean_whisper_text --

    #[test]
    fn clean_whisper_space_before_period() {
        assert_eq!(clean_whisper_text("test ."), "test.");
    }

    #[test]
    fn clean_whisper_contraction() {
        assert_eq!(clean_whisper_text("I 'm going"), "I'm going");
    }

    #[test]
    fn clean_whisper_comma() {
        assert_eq!(clean_whisper_text("Now , let"), "Now, let");
    }

    #[test]
    fn clean_whisper_multiple() {
        assert_eq!(
            clean_whisper_text("Hello , I 'm here . Great !"),
            "Hello, I'm here. Great!"
        );
    }

    #[test]
    fn clean_whisper_no_change() {
        assert_eq!(clean_whisper_text("no changes here"), "no changes here");
    }

    #[test]
    fn clean_whisper_preserves_spaces() {
        assert_eq!(clean_whisper_text("a b c"), "a b c");
    }

    // -- is_noise_marker --

    #[test]
    fn noise_marker_parenthesized() {
        assert!(is_noise_marker("[music]"));
        assert!(is_noise_marker("(buzzing)"));
        assert!(is_noise_marker("  [typing sounds]  "));
        assert!(!is_noise_marker("hello"));
        assert!(!is_noise_marker("[not closed"));
    }

    // -- snip --

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
}
