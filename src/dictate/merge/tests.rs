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
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "first second\n");
}

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
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
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
                old: "    pub timeout: u64,\n".to_string(),
                new: "    pub timeout: Duration,\n".to_string(),
            },
            Event::Words {
                offset_secs: 6.0,
                text: "to use Duration instead".to_string(),
            },
        ];
    let md = format_markdown(&mut events, SnipConfig::default());
    insta::assert_snapshot!(md);
}

#[test]
fn prose_after_diff() {
    let mut events = vec![
        Event::FileDiff {
            offset_secs: 0.0,
            path: "foo.rs".to_string(),
            old: "".to_string(),
            new: "new line\n".to_string(),
        },
        Event::Words {
            offset_secs: 1.0,
            text: "that was the change".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
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
    let md = format_markdown(&mut events, SnipConfig::default());
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

#[test]
fn snip_below_threshold_unchanged() {
    let text = "line1\nline2\nline3\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    assert_eq!(snip(text, cfg), text);
}

#[test]
fn snip_above_threshold_collapses() {
    // 6 lines, threshold 5 → snip with head=2, tail=1
    let text = "a\nb\nc\nd\ne\nf\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    let result = snip(text, cfg);
    assert_eq!(result, "a\nb\n// ... (3 lines omitted)\nf\n");
}

#[test]
fn snip_at_exact_threshold_unchanged() {
    let text = (1..=5).map(|i| format!("line{i}")).collect::<Vec<_>>().join("\n") + "\n";
    let cfg = SnipConfig {
        threshold: 5,
        head: 2,
        tail: 1,
    };
    assert_eq!(snip(&text, cfg), text);
}

#[test]
fn snip_applied_to_code_block() {
    // 25 lines of content → should be snipped with default config (threshold=20)
    let content: String = (1..=25).map(|i| format!("line {i}\n")).collect();
    let mut events = vec![Event::EditorSnapshot {
        offset_secs: 0.0,
        files: vec![],
        rendered: vec![RenderedFile {
            path: "big.rs".to_string(),
            content,
            first_line: 1,
        }],
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("// ... (10 lines omitted)"));
    assert!(md.contains("line 1\n"));
    assert!(md.contains("line 10\n"));
    assert!(md.contains("line 25\n"));
    assert!(!md.contains("line 11\n"));
}

#[test]
fn snip_applied_to_diff_block() {
    let new_content: String = (1..=25).map(|i| format!("line {i}\n")).collect();
    let mut events = vec![Event::FileDiff {
        offset_secs: 0.0,
        path: "big.rs".to_string(),
        old: String::new(),
        new: new_content,
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("// ... (10 lines omitted)"));
    assert!(md.contains("+line 1\n"));
    assert!(md.contains("+line 10\n"));
    assert!(md.contains("+line 25\n"));
    assert!(!md.contains("+line 11\n"));
}

#[test]
fn compress_consecutive_cursor_snapshots() {
    use crate::state::{FileEntry, Selection, Position, Line, Col};

    // Helper: cursor-only snapshot (all selections are cursor-like).
    let cursor_snap = |t: f64, path: &str| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection { start: pos, end: pos }],
            }],
            rendered: vec![RenderedFile {
                path: path.to_string(),
                content: "x\n".to_string(),
                first_line: 1,
            }],
        }
    };

    let mut events = vec![
        cursor_snap(1.0, "a.rs"),
        cursor_snap(2.0, "b.rs"),
        cursor_snap(3.0, "c.rs"),
        Event::Words {
            offset_secs: 4.0,
            text: "hello".to_string(),
        },
        cursor_snap(5.0, "d.rs"),
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    // Only c.rs (last cursor before words) and d.rs (after words) should appear.
    assert!(!md.contains("a.rs"), "a.rs should be compressed away");
    assert!(!md.contains("b.rs"), "b.rs should be compressed away");
    assert!(md.contains("c.rs"), "c.rs should be kept (last in run)");
    assert!(md.contains("d.rs"), "d.rs should be kept");
    assert!(md.contains("hello"));
}

#[test]
fn compress_preserves_selection_snapshots() {
    use crate::state::{FileEntry, Selection, Position, Line, Col};

    let cursor_snap = |t: f64, path: &str| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection { start: pos, end: pos }],
            }],
            rendered: vec![RenderedFile {
                path: path.to_string(),
                content: "x\n".to_string(),
                first_line: 1,
            }],
        }
    };

    // Snapshot with a real selection (highlight).
    let selection_snap = |t: f64, path: &str| {
        let start = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        let end = Position {
            line: Line::new(5).unwrap(),
            col: Col::new(10).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection { start, end }],
            }],
            rendered: vec![RenderedFile {
                path: path.to_string(),
                content: "selected\n".to_string(),
                first_line: 1,
            }],
        }
    };

    // cursor, selection, cursor — the selection must survive even though
    // it's between two cursor-only snapshots with no words.
    // After merge_adjacent, the selection and cursor snapshots are combined
    // into one snapshot containing both files.
    let mut events = vec![
        cursor_snap(1.0, "a.rs"),
        selection_snap(2.0, "b.rs"),
        cursor_snap(3.0, "c.rs"),
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(!md.contains("a.rs"), "cursor-only a.rs should be compressed");
    assert!(md.contains("b.rs"), "selection b.rs must be preserved");
    assert!(md.contains("c.rs"), "c.rs is last cursor, should be kept");

    // Both b.rs and c.rs should appear in one fenced block (merged snapshot),
    // so there should be exactly one ``` ... ``` ... ``` ... ``` pair sequence.
    let fence_count = md.matches("```\n").count();
    // Two files × (opening + closing) = 4 fence lines
    assert_eq!(fence_count, 4, "both files in single merged snapshot");
}

#[test]
fn compress_keeps_diffs_between_snapshots() {
    use crate::state::{FileEntry, Selection, Position, Line, Col};

    let cursor_snap = |t: f64, path: &str| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection { start: pos, end: pos }],
            }],
            rendered: vec![RenderedFile {
                path: path.to_string(),
                content: "x\n".to_string(),
                first_line: 1,
            }],
        }
    };

    let mut events = vec![
        cursor_snap(1.0, "a.rs"),
        Event::FileDiff {
            offset_secs: 2.0,
            path: "changed.rs".to_string(),
            old: "".to_string(),
            new: "added\n".to_string(),
        },
        cursor_snap(3.0, "b.rs"),
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(!md.contains("a.rs"), "a.rs should be compressed away");
    assert!(md.contains("changed.rs"), "diff should be kept");
    assert!(md.contains("b.rs"), "b.rs should be kept (last in run)");
}

#[test]
fn merge_snapshots_union() {
    use crate::state::{Col, FileEntry, Line, Position, Selection};

    let selection_snap = |t: f64, path: &str, content: &str| {
        let start = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        let end = Position {
            line: Line::new(5).unwrap(),
            col: Col::new(10).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection { start, end }],
            }],
            rendered: vec![RenderedFile {
                path: path.to_string(),
                content: content.to_string(),
                first_line: 1,
            }],
        }
    };

    // Two selection snapshots with no words — both files should appear.
    let mut events = vec![
        selection_snap(1.0, "a.rs", "fn a()\n"),
        selection_snap(2.0, "b.rs", "fn b()\n"),
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("a.rs"), "a.rs should be in merged snapshot");
    assert!(md.contains("fn a()"), "a.rs content preserved");
    assert!(md.contains("b.rs"), "b.rs should be in merged snapshot");
    assert!(md.contains("fn b()"), "b.rs content preserved");
}

#[test]
fn merge_diffs_net_change() {
    // File changes A→B then B→C between utterances.
    // The merged output should show the net diff A→C.
    let mut events = vec![
        Event::Words {
            offset_secs: 0.0,
            text: "before".to_string(),
        },
        Event::FileDiff {
            offset_secs: 1.0,
            path: "f.rs".to_string(),
            old: "aaa\nbbb\nccc\n".to_string(),
            new: "aaa\nBBB\nccc\n".to_string(),
        },
        Event::FileDiff {
            offset_secs: 2.0,
            path: "f.rs".to_string(),
            old: "aaa\nBBB\nccc\n".to_string(),
            new: "aaa\nBBB\nCCC\n".to_string(),
        },
        Event::Words {
            offset_secs: 3.0,
            text: "after".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    // Net diff should show bbb→BBB and ccc→CCC in one block.
    assert!(md.contains("-bbb"), "net diff should delete bbb");
    assert!(md.contains("+BBB"), "net diff should insert BBB");
    assert!(md.contains("-ccc"), "net diff should delete ccc");
    assert!(md.contains("+CCC"), "net diff should insert CCC");
    // Should be a single diff block, not two.
    let diff_fence_count = md.matches("```diff").count();
    assert_eq!(diff_fence_count, 1, "should produce one merged diff block");
}

#[test]
fn merge_diffs_cancelled_change_disappears() {
    // File changes A→B then B→A (reverted). Net diff is empty.
    let mut events = vec![
        Event::FileDiff {
            offset_secs: 1.0,
            path: "f.rs".to_string(),
            old: "original\n".to_string(),
            new: "changed\n".to_string(),
        },
        Event::FileDiff {
            offset_secs: 2.0,
            path: "f.rs".to_string(),
            old: "changed\n".to_string(),
            new: "original\n".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        !md.contains("```diff"),
        "reverted change should produce no diff block"
    );
}

#[test]
fn snip_disabled_with_large_threshold() {
    let content: String = (1..=100).map(|i| format!("line {i}\n")).collect();
    let cfg = SnipConfig {
        threshold: 1000,
        head: 10,
        tail: 5,
    };
    let mut events = vec![Event::EditorSnapshot {
        offset_secs: 0.0,
        files: vec![],
        rendered: vec![RenderedFile {
            path: "big.rs".to_string(),
            content: content.clone(),
            first_line: 1,
        }],
    }];
    let md = format_markdown(&mut events, cfg);
    assert!(!md.contains("omitted"));
    assert!(md.contains("line 50\n"));
}
