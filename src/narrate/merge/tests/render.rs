use super::super::*;

use crate::narrate::render::{SnipConfig, format_markdown};

/// Convert seconds to a UTC timestamp (for test brevity).
fn ts(secs: f64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds((secs * 1000.0) as i64)
}

/// Speech-only events render as a single prose line.
#[test]
fn words_only() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "Please".to_string(),
        },
        Event::Words {
            timestamp: ts(0.5),
            text: "look at this".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "Please look at this\n");
}

/// Words interleaved with editor snapshots render as prose + fenced code blocks.
#[test]
fn words_with_code() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "Look at this function".to_string(),
        },
        Event::EditorSnapshot {
            timestamp: ts(1.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "src/main.rs".to_string(),
                content: "fn main() {\n    println!(\"hello\");\n}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
        Event::Words {
            timestamp: ts(2.0),
            text: "and refactor it".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
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

/// A file diff renders as a fenced diff block with +/- lines.
#[test]
fn diff_event() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "I just changed this".to_string(),
        },
        Event::FileDiff {
            timestamp: ts(1.0),
            path: "src/lib.rs".to_string(),
            old: "    pub timeout: u64,\n".to_string(),
            new: "    pub timeout: Duration,\n".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
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

/// Events are sorted chronologically regardless of input order.
#[test]
fn chronological_ordering() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(2.0),
            text: "second".to_string(),
        },
        Event::Words {
            timestamp: ts(0.0),
            text: "first".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "first second\n");
}

/// unified_diff produces standard +/- output for changed lines.
#[test]
fn unified_diff_basic() {
    let old = "line1\nline2\nline3\n";
    let new = "line1\nchanged\nline3\n";
    let diff = unified_diff(old, new);
    assert!(diff.contains("-line2"));
    assert!(diff.contains("+changed"));
    assert!(diff.contains(" line1"));
}

/// An empty event list produces an empty string.
#[test]
fn empty_events() {
    let mut events: Vec<Event> = vec![];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "");
}

/// A snapshot with no words renders as a fenced code block with line number.
#[test]
fn code_only_no_prose() {
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![CapturedRegion {
            path: "src/lib.rs".to_string(),
            content: "pub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n".to_string(),
            first_line: 42,
            selections: vec![],
            language: None,
        }],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    let expected =
        "\n```\n// src/lib.rs:42\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n```\n";
    assert_eq!(md, expected);
}

/// A snapshot with multiple files renders each in its own fenced block.
#[test]
fn multiple_files_in_snapshot() {
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![
            CapturedRegion {
                path: "src/a.py".to_string(),
                content: "def foo():\n    pass\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            },
            CapturedRegion {
                path: "src/b.js".to_string(),
                content: "const x = 1;\n".to_string(),
                first_line: 10,
                selections: vec![],
                language: None,
            },
        ],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("```\n// src/a.py:1"));
    assert!(md.contains("```\n// src/b.js:10"));
}

/// Full rendering scenario: prose, code, prose, code, prose, diff, prose (snapshot test).
#[test]
fn full_scenario_snapshot() {
    let mut events = vec![
            Event::Words {
                timestamp: ts(0.0),
                text: "Please look at this function".to_string(),
            },
            Event::EditorSnapshot {
                timestamp: ts(1.5),
                files: vec![],
                regions: vec![CapturedRegion {
                    path: "src/main.rs".to_string(),
                    content: "fn parse_config(path: &Path) -> Result<Config> {\n    let raw = std::fs::read_to_string(path)?;\n    toml::from_str(&raw)\n}\n".to_string(),
                    first_line: 42,
                    selections: vec![],
                    language: None,
                }],
            },
            Event::Words {
                timestamp: ts(3.0),
                text: "and refactor it to use this struct".to_string(),
            },
            Event::EditorSnapshot {
                timestamp: ts(4.0),
                files: vec![],
                regions: vec![CapturedRegion {
                    path: "src/lib.rs".to_string(),
                    content: "pub struct Config {\n    pub name: String,\n    pub timeout: Duration,\n}\n".to_string(),
                    first_line: 8,
                    selections: vec![],
                    language: None,
                }],
            },
            Event::Words {
                timestamp: ts(5.0),
                text: "I just changed the timeout field".to_string(),
            },
            Event::FileDiff {
                timestamp: ts(5.5),
                path: "src/lib.rs".to_string(),
                old: "    pub timeout: u64,\n".to_string(),
                new: "    pub timeout: Duration,\n".to_string(),
            },
            Event::Words {
                timestamp: ts(6.0),
                text: "to use Duration instead".to_string(),
            },
        ];
    let md = format_markdown(&mut events, SnipConfig::default());
    insta::assert_snapshot!(md);
}

/// Prose following a diff block is separated by a blank line.
#[test]
fn prose_after_diff() {
    let mut events = vec![
        Event::FileDiff {
            timestamp: ts(0.0),
            path: "foo.rs".to_string(),
            old: "".to_string(),
            new: "new line\n".to_string(),
        },
        Event::Words {
            timestamp: ts(1.0),
            text: "that was the change".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("```diff\n// foo.rs\n+new line\n```\n"));
    assert!(md.contains("\nthat was the change\n"));
}

/// Content without a trailing newline still produces a properly closed fence.
#[test]
fn content_without_trailing_newline() {
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![CapturedRegion {
            path: "f.rs".to_string(),
            content: "no trailing newline".to_string(),
            first_line: 1,
            selections: vec![],
            language: None,
        }],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    // Should still end with closing fence + newline
    assert!(md.ends_with("no trailing newline\n```\n"));
}

/// Whisper artifacts (spaces before punctuation) are cleaned within a segment.
#[test]
fn whisper_cleanup_intra_segment() {
    // Within a single segment, spaces before punctuation are cleaned
    let mut events = vec![Event::Words {
        timestamp: ts(0.0),
        text: "I 'm going to fix this .".to_string(),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "I'm going to fix this.\n");
}

/// Whisper artifacts are cleaned across segment boundaries.
#[test]
fn whisper_cleanup_cross_segment() {
    // Punctuation as separate segments (max_len=1 mode)
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "function".to_string(),
        },
        Event::Words {
            timestamp: ts(0.1),
            text: ".".to_string(),
        },
        Event::Words {
            timestamp: ts(0.2),
            text: "I".to_string(),
        },
        Event::Words {
            timestamp: ts(0.3),
            text: "'m".to_string(),
        },
        Event::Words {
            timestamp: ts(0.4),
            text: "wondering".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "function. I'm wondering\n");
}

/// Bracketed noise markers like [typing sounds] are filtered from output.
#[test]
fn noise_markers_filtered() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "hello".to_string(),
        },
        Event::Words {
            timestamp: ts(0.5),
            text: "[typing sounds]".to_string(),
        },
        Event::Words {
            timestamp: ts(1.0),
            text: "world".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "hello world\n");
}

/// Code blocks exceeding the snip threshold are collapsed with an omission marker.
#[test]
fn snip_applied_to_code_block() {
    // 25 lines of content → snipped with default config (threshold=5, head=3, tail=2)
    let content: String = (1..=25).map(|i| format!("line {i}\n")).collect();
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![CapturedRegion {
            path: "big.rs".to_string(),
            content,
            first_line: 1,
            selections: vec![],
            language: None,
        }],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("// ... (lines 4-23 omitted)"));
    assert!(md.contains("line 1\n"));
    assert!(md.contains("line 3\n"));
    assert!(!md.contains("line 4\n"));
    assert!(md.contains("line 24\n"));
    assert!(md.contains("line 25\n"));
}

/// Diffs are never snipped (transient on-disk state cannot be reconstructed).
#[test]
fn diff_block_not_snipped() {
    let new_content: String = (1..=25).map(|i| format!("line {i}\n")).collect();
    let mut events = vec![Event::FileDiff {
        timestamp: ts(0.0),
        path: "big.rs".to_string(),
        old: String::new(),
        new: new_content,
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    // All 25 lines should be present, no omission marker.
    assert!(!md.contains("omitted"));
    for i in 1..=25 {
        assert!(md.contains(&format!("+line {i}\n")));
    }
}

/// Code-only scenario: no words at all, multiple snapshots and diffs (snapshot test).
#[test]
fn code_only_scenario_snapshot() {
    let mut events = vec![
        Event::EditorSnapshot {
            timestamp: ts(0.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "src/config.rs".to_string(),
                content: "pub struct Config {\n    pub name: String,\n}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
        Event::FileDiff {
            timestamp: ts(1.0),
            path: "src/config.rs".to_string(),
            old: "pub struct Config {\n    pub name: String,\n}\n".to_string(),
            new: "pub struct Config {\n    pub name: String,\n    pub port: u16,\n}\n".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    insta::assert_snapshot!(md);
}

/// Speech interleaved with multiple diffs to different files (snapshot test).
#[test]
fn multiple_diffs_snapshot() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "I renamed the field in both files".to_string(),
        },
        Event::FileDiff {
            timestamp: ts(1.0),
            path: "src/api.rs".to_string(),
            old: "    timeout: u64,\n".to_string(),
            new: "    timeout: Duration,\n".to_string(),
        },
        Event::FileDiff {
            timestamp: ts(1.5),
            path: "src/client.rs".to_string(),
            old: "    connect_timeout: u64,\n".to_string(),
            new: "    connect_timeout: Duration,\n".to_string(),
        },
        Event::Words {
            timestamp: ts(2.0),
            text: "to use Duration instead of raw u64".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    insta::assert_snapshot!(md);
}

/// A large snip threshold effectively disables snipping.
#[test]
fn snip_disabled_with_large_threshold() {
    let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    let cfg = SnipConfig {
        threshold: 1000,
        head: 10,
        tail: 5,
    };
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![CapturedRegion {
            path: "big.rs".to_string(),
            content: content.clone(),
            first_line: 1,
            selections: vec![],
            language: None,
        }],
    }];
    let md = format_markdown(&mut events, cfg);
    assert!(!md.contains("omitted"));
    assert!(md.contains("line 50\n"));
}

// ── Language tag rendering ────────────────────────────────────────────────

/// Fence includes language tag when `language` is Some.
#[test]
fn language_tag_in_fence() {
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![CapturedRegion {
            path: "src/main.rs".to_string(),
            content: "fn main() {}\n".to_string(),
            first_line: 1,
            selections: vec![],
            language: Some("rust".to_string()),
        }],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        md.contains("```rust\n"),
        "fence should include language tag: {md:?}"
    );
}

/// Bare fence when `language` is None.
#[test]
fn bare_fence_when_no_language() {
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        files: vec![],
        regions: vec![CapturedRegion {
            path: "src/main.rs".to_string(),
            content: "fn main() {}\n".to_string(),
            first_line: 1,
            selections: vec![],
            language: None,
        }],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        md.contains("```\n// src/main.rs"),
        "fence should be bare when no language: {md:?}"
    );
    assert!(
        !md.contains("```rust"),
        "no language tag should appear: {md:?}"
    );
}
