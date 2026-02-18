//! Chronological merge of transcription, editor snapshots, and file diffs.
//!
//! Sorts all captured events by wall-clock time and produces a markdown
//! document interleaving prose (from speech) with fenced code blocks
//! (from editor navigation) and fenced diff blocks (from file changes).

use crate::state::FileEntry;

/// Controls collapsing of large code snippets and diffs.
///
/// When a fenced block exceeds `threshold` lines, only the first `head`
/// and last `tail` lines are kept, with a `// ... (N lines omitted)` marker
/// in between.
#[derive(Debug, Clone, Copy)]
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
            threshold: 20,
            head: 10,
            tail: 5,
        }
    }
}

/// Collapse a multi-line string if it exceeds the snip threshold.
///
/// Returns the original string unchanged if it fits, otherwise keeps the
/// first `head` and last `tail` lines with an omission marker.
fn snip(text: &str, cfg: SnipConfig) -> String {
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
    out.push_str(&format!("// ... ({omitted} lines omitted)\n"));
    for line in &lines[lines.len() - cfg.tail..] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// A timestamped event from one of the three capture streams.
#[derive(Debug, Clone)]
pub enum Event {
    /// A transcribed word or group of words.
    Words {
        /// Seconds from recording start.
        offset_secs: f64,
        /// The transcribed text.
        text: String,
    },
    /// An editor state snapshot captured when selections changed.
    EditorSnapshot {
        /// Seconds from recording start.
        offset_secs: f64,
        /// Files with their selections at this point (retained for debugging/archive).
        #[allow(dead_code)]
        files: Vec<FileEntry>,
        /// Pre-rendered view content (from `render_json`).
        rendered: Vec<RenderedFile>,
    },
    /// A file diff captured when file content changed.
    FileDiff {
        /// Seconds from recording start.
        offset_secs: f64,
        /// Relative path of the changed file.
        path: String,
        /// Unified diff content.
        diff: String,
    },
}

/// Pre-rendered file view for an editor snapshot.
#[derive(Debug, Clone)]
pub struct RenderedFile {
    /// Display path (relative).
    pub path: String,
    /// Rendered content with selection markers.
    pub content: String,
    /// First visible line number.
    pub first_line: u32,
}

impl Event {
    /// Sort key: seconds from recording start.
    pub fn offset_secs(&self) -> f64 {
        match self {
            Event::Words { offset_secs, .. }
            | Event::EditorSnapshot { offset_secs, .. }
            | Event::FileDiff { offset_secs, .. } => *offset_secs,
        }
    }
}

/// Produce a unified diff between two strings using the `similar` crate.
#[cfg(feature = "dictate")]
pub fn unified_diff(old: &str, new: &str) -> String {
    use similar::{ChangeTag, TextDiff};

    let diff = TextDiff::from_lines(old, new);
    let mut out = String::new();

    for change in diff.iter_all_changes() {
        let sign = match change.tag() {
            ChangeTag::Delete => "-",
            ChangeTag::Insert => "+",
            ChangeTag::Equal => " ",
        };
        out.push_str(sign);
        out.push_str(change.as_str().unwrap_or(""));
        if !change.as_str().unwrap_or("").ends_with('\n') {
            out.push('\n');
        }
    }

    out
}

#[cfg(not(feature = "dictate"))]
pub fn unified_diff(_old: &str, _new: &str) -> String {
    String::new()
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

/// Merge all events chronologically and format as markdown.
///
/// The output interleaves prose text with fenced code/diff blocks:
/// - Words become flowing prose text
/// - Editor snapshots become fenced code blocks with `// path:line` headers
/// - File diffs become fenced diff blocks with `// path` headers
pub fn format_markdown(events: &mut [Event], snip_cfg: SnipConfig) -> String {
    events.sort_by(|a, b| {
        a.offset_secs()
            .partial_cmp(&b.offset_secs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

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
                    let snipped = snip(&file.content, snip_cfg);
                    out.push_str("```\n");
                    out.push_str(&format!("// {}:{}\n", file.path, file.first_line));
                    out.push_str(&snipped);
                    if !snipped.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n");
                }
            }
            Event::FileDiff { path, diff, .. } => {
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                let snipped = snip(diff, snip_cfg);
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

#[cfg(test)]
mod tests;
