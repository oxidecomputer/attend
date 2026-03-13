//! Markdown rendering for merged narration events.
//!
//! Takes a sorted/compressed event list and produces a markdown document
//! interleaving prose (from speech) with fenced code blocks (from editor
//! navigation) and fenced diff blocks (from file changes).

use std::fmt;

use serde::{Deserialize, Serialize};

use super::merge::{self, Event, RedactedKind};

/// Controls how clipboard images are rendered in markdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RenderMode {
    /// Render clipboard images as `![clipboard](path)` — for agent consumption.
    /// The agent can `Read` the file directly.
    Agent,
    /// Render clipboard images as `![clipboard](data:image/png;base64,...)`
    /// — self-contained for clipboard paste.
    Yank,
}

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
///
/// This is an intentional allowlist rather than a Unicode category check.
/// The `Po` (Other Punctuation) general category is a grab-bag that includes
/// characters which should *not* fuse left (`#`, `&`, `*`, `@`, `\`, `/`,
/// `¡`, `¿`), so no combination of Unicode categories works as a predicate.
///
/// Parakeet's 8192-token SentencePiece vocabulary contains only ASCII
/// punctuation (no `Pe`/`Pf`/`Pi`/`Ps` characters at all), so the fuse-left
/// Po set is just `. , : ! ? ' %`. The extra entries (`; " ) ] }`) cover
/// Whisper output and are harmless for Parakeet.
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

/// Format a single `Redacted` event's label (without the ✂ prefix).
fn redacted_label(kind: &RedactedKind, keys: &[String]) -> String {
    let count = keys.len();
    match (kind, count) {
        (RedactedKind::EditorSnapshot, 1) => "file".to_string(),
        (RedactedKind::EditorSnapshot, n) => format!("{n} files"),
        (RedactedKind::FileDiff, 1) => "edit".to_string(),
        (RedactedKind::FileDiff, n) => format!("{n} edits"),
        (RedactedKind::ShellCommand, 1) => "command".to_string(),
        (RedactedKind::ShellCommand, n) => format!("{n} commands"),
    }
}

/// Emit the blank-line separator that precedes every non-prose block.
///
/// Unconditionally writes a newline (the blank line before a fenced block
/// or blockquote), then sets `has_content = true`.
fn start_block(out: &mut dyn fmt::Write, has_content: &mut bool) -> fmt::Result {
    writeln!(out)?;
    *has_content = true;
    Ok(())
}

/// Render a sorted/compressed event list as markdown into a [`fmt::Write`]
/// sink.
///
/// The output interleaves prose text with fenced code/diff blocks:
/// - Words become flowing prose text
/// - Editor snapshots become fenced code blocks with `` `path:line`: `` labels
/// - File diffs become fenced diff blocks with `` `path`: `` labels
/// - Shell commands become fenced code blocks with `$ ` command prefix
/// - External selections become attributed blockquotes
/// - Browser selections become link-attributed blockquotes
/// - Redacted events become `✂` markers, comma-separated when adjacent
fn render_markdown_to(
    out: &mut dyn fmt::Write,
    events: &[Event],
    snip_cfg: SnipConfig,
    mode: RenderMode,
) -> fmt::Result {
    let mut in_prose = false;
    // Tracks whether any content has been written, so the first block
    // does not emit a leading blank line.
    let mut has_content = false;
    let mut i = 0;

    while i < events.len() {
        let event = &events[i];
        match event {
            Event::Words { text, .. } => {
                let cleaned = clean_whisper_text(text);
                if cleaned.is_empty() || is_noise_marker(&cleaned) {
                    i += 1;
                    continue;
                }
                if !in_prose && has_content {
                    writeln!(out)?;
                }
                // Skip the space before punctuation that attaches to previous word
                if in_prose && !starts_with_punctuation(&cleaned) {
                    write!(out, " ")?;
                }
                write!(out, "{cleaned}")?;
                in_prose = true;
                has_content = true;
            }
            Event::EditorSnapshot { regions, .. } => {
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                // EditorSnapshot deliberately does not use start_block():
                // each region emits its own blank-line separator so that
                // multi-region snapshots are visually distinct.
                for region in regions {
                    writeln!(out)?;
                    has_content = true;
                    let annotated = crate::view::apply_markers(
                        &region.content,
                        region.first_line,
                        &region.selections,
                    );
                    let snipped = snip(&annotated, snip_cfg, Some(region.first_line as usize));
                    writeln!(out, "`{}:{}`:", region.path, region.first_line)?;
                    match &region.language {
                        Some(lang) => writeln!(out, "```{lang}")?,
                        None => writeln!(out, "```")?,
                    }
                    write!(out, "{snipped}")?;
                    if !snipped.ends_with('\n') {
                        writeln!(out)?;
                    }
                    writeln!(out, "```")?;
                }
            }
            Event::FileDiff { path, old, new, .. } => {
                if old == new {
                    i += 1;
                    continue;
                }
                let diff = merge::unified_diff(old, new);
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                start_block(out, &mut has_content)?;
                writeln!(out, "`{path}`:")?;
                writeln!(out, "```diff")?;
                // Diffs are not snipped: they represent transient on-disk state
                // that cannot be reconstructed after the fact.
                write!(out, "{diff}")?;
                if !diff.ends_with('\n') {
                    writeln!(out)?;
                }
                writeln!(out, "```")?;
            }
            Event::ExternalSelection {
                app,
                window_title,
                text,
                ..
            } => {
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                start_block(out, &mut has_content)?;
                // Render as attribution label above a blockquote.
                // External selections are not snipped: they represent ephemeral
                // state (accessibility API) that cannot be reconstructed.
                if window_title.is_empty() {
                    writeln!(out, "{app}:")?;
                } else {
                    writeln!(out, "{app}: {window_title}:")?;
                }
                for line in text.trim().lines() {
                    writeln!(out, "> {line}")?;
                }
            }
            Event::BrowserSelection {
                url, title, text, ..
            } => {
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                start_block(out, &mut has_content)?;
                // Browser selections are not snipped: they represent ephemeral
                // page content that cannot be reconstructed after navigation.
                // The text field contains markdown converted from HTML by the
                // browser bridge (via htmd).
                if title.is_empty() {
                    writeln!(out, "<{url}>:")?;
                } else {
                    writeln!(out, "[{title}]({url}):")?;
                }
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    for line in trimmed.lines() {
                        writeln!(out, "> {line}")?;
                    }
                }
            }
            Event::ShellCommand {
                shell,
                command,
                cwd,
                exit_status,
                duration_secs,
                ..
            } => {
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                start_block(out, &mut has_content)?;
                // Show the working directory when it's not the project root.
                if !cwd.is_empty() && cwd != "." {
                    writeln!(out, "In `{cwd}/`:")?;
                }
                writeln!(out, "```{shell}")?;
                write!(out, "$ {command}")?;
                // Append exit status and duration as a trailing shell comment
                // for token efficiency. Omit when exit 0 and < 1s (trivial).
                match (exit_status, duration_secs) {
                    (Some(code), Some(dur)) if *code != 0 || *dur >= 1.0 => {
                        write!(out, "  # exit {code}, {dur:.1}s")?;
                    }
                    (Some(code), None) if *code != 0 => {
                        write!(out, "  # exit {code}")?;
                    }
                    (None, _) => {
                        // Preexec (command still running): no comment.
                    }
                    _ => {}
                }
                writeln!(out)?;
                writeln!(out, "```")?;
            }
            Event::ClipboardSelection { content, .. } => {
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                start_block(out, &mut has_content)?;
                match content {
                    merge::ClipboardContent::Text { text } => {
                        // Plain blockquote with no attribution.
                        // Clipboard selections are not snipped (ephemeral state).
                        for line in text.trim().lines() {
                            writeln!(out, "> {line}")?;
                        }
                    }
                    merge::ClipboardContent::Image { path } => match mode {
                        RenderMode::Agent => {
                            writeln!(out, "![clipboard]({path})")?;
                        }
                        RenderMode::Yank => {
                            if let Ok(bytes) = std::fs::read(path) {
                                use base64::Engine as _;
                                let encoded =
                                    base64::engine::general_purpose::STANDARD.encode(&bytes);
                                writeln!(out, "![clipboard](data:image/png;base64,{encoded})")?;
                            } else {
                                writeln!(out, "[clipboard image unavailable]")?;
                            }
                        }
                    },
                }
            }
            Event::Redacted { kind, keys, .. } => {
                if in_prose {
                    writeln!(out)?;
                    in_prose = false;
                }
                start_block(out, &mut has_content)?;
                // Collect consecutive Redacted events onto one comma-separated line.
                let mut labels = vec![redacted_label(kind, keys)];
                while i + 1 < events.len() {
                    if let Event::Redacted {
                        kind: k, keys: ks, ..
                    } = &events[i + 1]
                    {
                        labels.push(redacted_label(k, ks));
                        i += 1;
                    } else {
                        break;
                    }
                }
                writeln!(out, "\u{2702} {}", labels.join(", "))?;
            }
        }
        i += 1;
    }

    if in_prose {
        writeln!(out)?;
    }

    Ok(())
}

/// Render a sorted/compressed event list as markdown.
///
/// Convenience wrapper around [`render_markdown_to`] that writes into a
/// `String` (which implements [`fmt::Write`]).
pub fn render_markdown(events: &[Event], snip_cfg: SnipConfig, mode: RenderMode) -> String {
    let mut out = String::new();
    // Writing to a String is infallible, so unwrap is safe.
    render_markdown_to(&mut out, events, snip_cfg, mode).unwrap();
    out
}

/// Merge all events chronologically and format as markdown.
///
/// Convenience function that calls [`merge::compress_and_merge`] followed by
/// [`render_markdown`]. Used primarily in tests.
#[cfg(test)]
pub fn format_markdown(events: &mut Vec<Event>, snip_cfg: SnipConfig) -> String {
    merge::compress_and_merge(events);
    render_markdown(events, snip_cfg, RenderMode::Agent)
}

#[cfg(test)]
mod tests;
