use anyhow::Context;

use crate::state::{FileEntry, Selection};

/// Parse compact format into FileEntry list.
///
/// Tokens matching `\d+:\d+(-\d+:\d+)?` (with optional trailing comma) are
/// positions; everything else starts a new file path.
pub fn parse_compact(input: &str) -> anyhow::Result<Vec<FileEntry>> {
    let tokens = tokenize(input);
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_sels: Vec<Selection> = Vec::new();

    for token in tokens {
        // Strip trailing comma for position detection
        let stripped = token.strip_suffix(',').unwrap_or(&token);

        if is_position(stripped) {
            let sel: Selection = stripped
                .parse()
                .with_context(|| format!("bad position: {stripped}"))?;
            current_sels.push(sel);
        } else {
            // New path token. Flush previous file if it had positions.
            // If previous had no positions, it might be part of a
            // space-containing path — concatenate.
            match current_path.take() {
                Some(prev) if current_sels.is_empty() => {
                    // Concatenation heuristic: prev had no positions,
                    // so join with space.
                    current_path = Some(format!("{prev} {token}"));
                }
                Some(prev) => {
                    entries.push(FileEntry {
                        path: prev.into(),
                        selections: std::mem::take(&mut current_sels),
                    });
                    current_path = Some(token);
                }
                None => {
                    current_path = Some(token);
                }
            }
        }
    }

    // Flush last file
    if let Some(path) = current_path
        && !current_sels.is_empty()
    {
        entries.push(FileEntry {
            path: path.into(),
            selections: current_sels,
        });
    }

    Ok(entries)
}

/// Check if a token looks like a position: `\d+:\d+` or `\d+:\d+-\d+:\d+`.
fn is_position(s: &str) -> bool {
    if let Some((left, right)) = s.split_once('-') {
        is_line_col(left) && is_line_col(right)
    } else {
        is_line_col(s)
    }
}

fn is_line_col(s: &str) -> bool {
    let Some((line, col)) = s.split_once(':') else {
        return false;
    };
    !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
        && !col.is_empty()
        && col.bytes().all(|b| b.is_ascii_digit())
}

/// Tokenize input respecting double-quoted strings.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' {
            // Quoted token
            chars.next(); // consume opening quote
            let mut token = String::new();
            while let Some(&ch) = chars.peek() {
                if ch == '"' {
                    chars.next(); // consume closing quote
                    break;
                }
                token.push(ch);
                chars.next();
            }
            tokens.push(token);
        } else {
            // Unquoted token — until whitespace
            let mut token = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                token.push(ch);
                chars.next();
            }
            tokens.push(token);
        }
    }

    tokens
}
