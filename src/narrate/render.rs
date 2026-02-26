//! Markdown rendering for merged narration events.
//!
//! Takes a sorted/compressed event list and produces a markdown document
//! interleaving prose (from speech) with fenced code blocks (from editor
//! navigation) and fenced diff blocks (from file changes).

use serde::{Deserialize, Serialize};

use super::merge::{self, Event, RedactedKind};

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

/// Render a sorted/compressed event list as markdown.
///
/// The output interleaves prose text with fenced code/diff blocks:
/// - Words become flowing prose text
/// - Editor snapshots become fenced code blocks with `` `path:line`: `` labels
/// - File diffs become fenced diff blocks with `` `path`: `` labels
/// - Shell commands become fenced code blocks with `$ ` command prefix
/// - External selections become attributed blockquotes
/// - Browser selections become link-attributed blockquotes
/// - Redacted events become `✂` markers, comma-separated when adjacent
pub fn render_markdown(events: &[Event], snip_cfg: SnipConfig) -> String {
    let mut out = String::new();
    let mut in_prose = false;
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
                    out.push_str(&format!("`{}:{}`:\n", region.path, region.first_line));
                    match &region.language {
                        Some(lang) => out.push_str(&format!("```{lang}\n")),
                        None => out.push_str("```\n"),
                    }
                    out.push_str(&snipped);
                    if !snipped.ends_with('\n') {
                        out.push('\n');
                    }
                    out.push_str("```\n");
                }
            }
            Event::FileDiff { path, old, new, .. } => {
                if old == new {
                    i += 1;
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
                out.push_str(&format!("`{path}`:\n"));
                out.push_str("```diff\n");
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
                // Render as attribution label above a blockquote.
                // External selections are not snipped: they represent ephemeral
                // state (accessibility API) that cannot be reconstructed.
                if window_title.is_empty() {
                    out.push_str(&format!("{app}:\n"));
                } else {
                    out.push_str(&format!("{app}: {window_title}:\n"));
                }
                for line in text.trim().lines() {
                    out.push_str(&format!("> {line}\n"));
                }
            }
            Event::BrowserSelection {
                url, title, text, ..
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
                // The text field contains markdown converted from HTML by the
                // browser bridge (via htmd).
                if title.is_empty() {
                    out.push_str(&format!("<{url}>:\n"));
                } else {
                    out.push_str(&format!("[{title}]({url}):\n"));
                }
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    for line in trimmed.lines() {
                        out.push_str(&format!("> {line}\n"));
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
                    out.push('\n');
                    in_prose = false;
                }
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
                // Show the working directory when it's not the project root.
                if !cwd.is_empty() && cwd != "." {
                    out.push_str(&format!("In `{cwd}/`:\n"));
                }
                out.push_str(&format!("```{shell}\n"));
                out.push_str(&format!("$ {command}"));
                // Append exit status and duration as a trailing shell comment
                // for token efficiency. Omit when exit 0 and < 1s (trivial).
                match (exit_status, duration_secs) {
                    (Some(code), Some(dur)) if *code != 0 || *dur >= 1.0 => {
                        out.push_str(&format!("  # exit {code}, {dur:.1}s"));
                    }
                    (Some(code), None) if *code != 0 => {
                        out.push_str(&format!("  # exit {code}"));
                    }
                    (None, _) => {
                        // Preexec (command still running): no comment.
                    }
                    _ => {}
                }
                out.push('\n');
                out.push_str("```\n");
            }
            Event::ClipboardSelection { .. } => {
                // Stub: to be implemented in Phase C.
                // Text renders as plain blockquote (no attribution).
                // Image renders as ![clipboard](path).
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
            }
            Event::Redacted { kind, keys, .. } => {
                if in_prose {
                    out.push('\n');
                    in_prose = false;
                }
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push('\n');
                }
                out.push('\n');
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
                out.push_str(&format!("\u{2702} {}\n", labels.join(", ")));
            }
        }
        i += 1;
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
mod tests;
