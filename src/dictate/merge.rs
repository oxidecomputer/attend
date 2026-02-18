//! Chronological merge of transcription, editor snapshots, and file diffs.
//!
//! Sorts all captured events by wall-clock time and produces a markdown
//! document interleaving prose (from speech) with fenced code blocks
//! (from editor navigation) and fenced diff blocks (from file changes).

use crate::state::FileEntry;

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
pub fn format_markdown(events: &mut [Event]) -> String {
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
                    out.push_str("```\n");
                    out.push_str(&format!("// {}:{}\n", file.path, file.first_line));
                    out.push_str(&file.content);
                    if !file.content.ends_with('\n') {
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
                out.push_str("```diff\n");
                out.push_str(&format!("// {path}\n"));
                out.push_str(diff);
                if !diff.ends_with('\n') {
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
mod tests {
    use super::*;

    #[test]
    fn words_only() {
        let mut events = vec![
            Event::Words {
                offset_secs: 0.0,
                text: "Please".to_string(),
            },
            Event::Words {
                offset_secs: 0.5,
                text: "look at this".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        assert_eq!(md, "Please look at this\n");
    }

    #[test]
    fn words_with_code() {
        let mut events = vec![
            Event::Words {
                offset_secs: 0.0,
                text: "Look at this function".to_string(),
            },
            Event::EditorSnapshot {
                offset_secs: 1.0,
                files: vec![],
                rendered: vec![RenderedFile {
                    path: "src/main.rs".to_string(),
                    content: "fn main() {\n    println!(\"hello\");\n}\n".to_string(),
                    first_line: 1,
                }],
            },
            Event::Words {
                offset_secs: 2.0,
                text: "and refactor it".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        let expected = "\
Look at this function

```
// src/main.rs:1
fn main() {
    println!(\"hello\");
}
```

and refactor it
";
        assert_eq!(md, expected);
    }

    #[test]
    fn diff_event() {
        let mut events = vec![
            Event::Words {
                offset_secs: 0.0,
                text: "I just changed this".to_string(),
            },
            Event::FileDiff {
                offset_secs: 1.0,
                path: "src/lib.rs".to_string(),
                diff: "-    pub timeout: u64,\n+    pub timeout: Duration,\n".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        let expected = "\
I just changed this

```diff
// src/lib.rs
-    pub timeout: u64,
+    pub timeout: Duration,
```
";
        assert_eq!(md, expected);
    }

    #[test]
    fn chronological_ordering() {
        let mut events = vec![
            Event::Words {
                offset_secs: 2.0,
                text: "second".to_string(),
            },
            Event::Words {
                offset_secs: 0.0,
                text: "first".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        assert_eq!(md, "first second\n");
    }

    #[cfg(feature = "dictate")]
    #[test]
    fn unified_diff_basic() {
        let old = "line1\nline2\nline3\n";
        let new = "line1\nchanged\nline3\n";
        let diff = unified_diff(old, new);
        assert!(diff.contains("-line2"));
        assert!(diff.contains("+changed"));
        assert!(diff.contains(" line1"));
    }

    #[test]
    fn empty_events() {
        let mut events: Vec<Event> = vec![];
        let md = format_markdown(&mut events);
        assert_eq!(md, "");
    }

    #[test]
    fn code_only_no_prose() {
        let mut events = vec![Event::EditorSnapshot {
            offset_secs: 0.0,
            files: vec![],
            rendered: vec![RenderedFile {
                path: "src/lib.rs".to_string(),
                content: "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n".to_string(),
                first_line: 42,
            }],
        }];
        let md = format_markdown(&mut events);
        let expected =
            "\n```\n// src/lib.rs:42\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n```\n";
        assert_eq!(md, expected);
    }

    #[test]
    fn multiple_files_in_snapshot() {
        let mut events = vec![Event::EditorSnapshot {
            offset_secs: 0.0,
            files: vec![],
            rendered: vec![
                RenderedFile {
                    path: "src/a.py".to_string(),
                    content: "def foo():\n    pass\n".to_string(),
                    first_line: 1,
                },
                RenderedFile {
                    path: "src/b.js".to_string(),
                    content: "const x = 1;\n".to_string(),
                    first_line: 10,
                },
            ],
        }];
        let md = format_markdown(&mut events);
        assert!(md.contains("```\n// src/a.py:1"));
        assert!(md.contains("```\n// src/b.js:10"));
    }

    #[test]
    fn full_scenario_snapshot() {
        let mut events = vec![
            Event::Words {
                offset_secs: 0.0,
                text: "Please look at this function".to_string(),
            },
            Event::EditorSnapshot {
                offset_secs: 1.5,
                files: vec![],
                rendered: vec![RenderedFile {
                    path: "src/main.rs".to_string(),
                    content: "fn parse_config(path: &Path) -> Result<Config> {\n    let raw = std::fs::read_to_string(path)?;\n    toml::from_str(&raw)\n}\n".to_string(),
                    first_line: 42,
                }],
            },
            Event::Words {
                offset_secs: 3.0,
                text: "and refactor it to use this struct".to_string(),
            },
            Event::EditorSnapshot {
                offset_secs: 4.0,
                files: vec![],
                rendered: vec![RenderedFile {
                    path: "src/lib.rs".to_string(),
                    content: "pub struct Config {\n    pub name: String,\n    pub timeout: Duration,\n}\n".to_string(),
                    first_line: 8,
                }],
            },
            Event::Words {
                offset_secs: 5.0,
                text: "I just changed the timeout field".to_string(),
            },
            Event::FileDiff {
                offset_secs: 5.5,
                path: "src/lib.rs".to_string(),
                diff: "-    pub timeout: u64,\n+    pub timeout: Duration,\n".to_string(),
            },
            Event::Words {
                offset_secs: 6.0,
                text: "to use Duration instead".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        insta::assert_snapshot!(md);
    }

    #[test]
    fn prose_after_diff() {
        let mut events = vec![
            Event::FileDiff {
                offset_secs: 0.0,
                path: "foo.rs".to_string(),
                diff: "+new line\n".to_string(),
            },
            Event::Words {
                offset_secs: 1.0,
                text: "that was the change".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        assert!(md.contains("```diff\n// foo.rs\n+new line\n```\n"));
        assert!(md.contains("\nthat was the change\n"));
    }

    #[test]
    fn content_without_trailing_newline() {
        let mut events = vec![Event::EditorSnapshot {
            offset_secs: 0.0,
            files: vec![],
            rendered: vec![RenderedFile {
                path: "f.rs".to_string(),
                content: "no trailing newline".to_string(),
                first_line: 1,
            }],
        }];
        let md = format_markdown(&mut events);
        // Should still end with closing fence + newline
        assert!(md.ends_with("no trailing newline\n```\n"));
    }

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

    #[test]
    fn whisper_cleanup_intra_segment() {
        // Within a single segment, spaces before punctuation are cleaned
        let mut events = vec![Event::Words {
            offset_secs: 0.0,
            text: "I 'm going to fix this .".to_string(),
        }];
        let md = format_markdown(&mut events);
        assert_eq!(md, "I'm going to fix this.\n");
    }

    #[test]
    fn whisper_cleanup_cross_segment() {
        // Punctuation as separate segments (max_len=1 mode)
        let mut events = vec![
            Event::Words {
                offset_secs: 0.0,
                text: "function".to_string(),
            },
            Event::Words {
                offset_secs: 0.1,
                text: ".".to_string(),
            },
            Event::Words {
                offset_secs: 0.2,
                text: "I".to_string(),
            },
            Event::Words {
                offset_secs: 0.3,
                text: "'m".to_string(),
            },
            Event::Words {
                offset_secs: 0.4,
                text: "wondering".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        assert_eq!(md, "function. I'm wondering\n");
    }

    #[test]
    fn noise_markers_filtered() {
        let mut events = vec![
            Event::Words {
                offset_secs: 0.0,
                text: "hello".to_string(),
            },
            Event::Words {
                offset_secs: 0.5,
                text: "[typing sounds]".to_string(),
            },
            Event::Words {
                offset_secs: 1.0,
                text: "world".to_string(),
            },
        ];
        let md = format_markdown(&mut events);
        assert_eq!(md, "hello world\n");
    }

    #[test]
    fn noise_marker_parenthesized() {
        assert!(is_noise_marker("[music]"));
        assert!(is_noise_marker("(buzzing)"));
        assert!(is_noise_marker("  [typing sounds]  "));
        assert!(!is_noise_marker("hello"));
        assert!(!is_noise_marker("[not closed"));
    }
}
