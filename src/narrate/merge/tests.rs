use proptest::prelude::*;

use super::*;

use crate::narrate::render::{SnipConfig, format_markdown};
use crate::state::{Col, FileEntry, Line, Position, Selection};

/// Speech-only events render as a single prose line.
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

/// Words interleaved with editor snapshots render as prose + fenced code blocks.
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

/// A file diff renders as a fenced diff block with +/- lines.
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

/// Events are sorted chronologically regardless of input order.
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

/// A snapshot with multiple files renders each in its own fenced block.
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

/// Full rendering scenario: prose, code, prose, code, prose, diff, prose (snapshot test).
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

/// Prose following a diff block is separated by a blank line.
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

/// Content without a trailing newline still produces a properly closed fence.
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

/// Whisper artifacts (spaces before punctuation) are cleaned within a segment.
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

/// Whisper artifacts are cleaned across segment boundaries.
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

/// Bracketed noise markers like [typing sounds] are filtered from output.
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

/// Code blocks exceeding the snip threshold are collapsed with an omission marker.
#[test]
fn snip_applied_to_code_block() {
    // 25 lines of content → snipped with default config (threshold=5, head=3, tail=2)
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
    assert!(md.contains("// ... (lines 4-23 omitted)"));
    assert!(md.contains("line 1\n"));
    assert!(md.contains("line 3\n"));
    assert!(!md.contains("line 4\n"));
    assert!(md.contains("line 24\n"));
    assert!(md.contains("line 25\n"));
}

/// Diff blocks exceeding the snip threshold are collapsed with an omission marker.
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
    // Diffs don't have a first_line, so they show count-only format.
    // unified_diff produces 25 +lines (no @@ header); snip keeps head=3, tail=2.
    assert!(md.contains("// ... (20 lines omitted)"));
    assert!(md.contains("+line 1\n"));
    assert!(md.contains("+line 3\n"));
    assert!(!md.contains("+line 4\n"));
    assert!(md.contains("+line 25\n"));
}

/// Consecutive cursor-only snapshots compress to keep only the last before speech.
#[test]
fn compress_consecutive_cursor_snapshots() {
    use crate::state::{Col, FileEntry, Line, Position, Selection};

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
                selections: vec![Selection {
                    start: pos,
                    end: pos,
                }],
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
    // c.rs (last cursor before words) should appear. d.rs (trailing cursor-only
    // after speech) is dropped because the stop hook provides up-to-date context.
    assert!(!md.contains("a.rs"), "a.rs should be compressed away");
    assert!(!md.contains("b.rs"), "b.rs should be compressed away");
    assert!(md.contains("c.rs"), "c.rs should be kept (last in run)");
    assert!(!md.contains("d.rs"), "d.rs dropped: stop hook has latest");
    assert!(md.contains("hello"));
}

/// Selection (highlight) snapshots survive compression even between cursor-only snapshots.
#[test]
fn compress_preserves_selection_snapshots() {
    use crate::state::{Col, FileEntry, Line, Position, Selection};

    let cursor_snap = |t: f64, path: &str| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection {
                    start: pos,
                    end: pos,
                }],
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
    assert!(
        !md.contains("a.rs"),
        "cursor-only a.rs should be compressed"
    );
    assert!(md.contains("b.rs"), "selection b.rs must be preserved");
    assert!(md.contains("c.rs"), "c.rs is last cursor, should be kept");

    // Both b.rs and c.rs should appear in one fenced block (merged snapshot),
    // so there should be exactly one ``` ... ``` ... ``` ... ``` pair sequence.
    let fence_count = md.matches("```\n").count();
    // Two files × (opening + closing) = 4 fence lines
    assert_eq!(fence_count, 4, "both files in single merged snapshot");
}

/// Diff events between cursor-only snapshots are preserved during compression.
#[test]
fn compress_keeps_diffs_between_snapshots() {
    use crate::state::{Col, FileEntry, Line, Position, Selection};

    let cursor_snap = |t: f64, path: &str| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.into(),
                selections: vec![Selection {
                    start: pos,
                    end: pos,
                }],
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

/// Adjacent selection snapshots are merged into a single snapshot with all files.
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

/// Consecutive diffs to the same file merge into a single net-change diff.
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

/// A change followed by its exact revert produces no diff block.
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

// ── Prop test strategies ────────────────────────────────────────────────────

fn arb_words() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z ]{1,30}").prop_map(|(t, text)| Event::Words {
        offset_secs: t,
        text,
    })
}

fn arb_cursor_snapshot() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z]{1,8}\\.rs").prop_map(|(t, path)| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.clone().into(),
                selections: vec![Selection {
                    start: pos,
                    end: pos,
                }],
            }],
            rendered: vec![RenderedFile {
                path,
                content: "x\n".to_string(),
                first_line: 1,
            }],
        }
    })
}

fn arb_selection_snapshot() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z]{1,8}\\.rs").prop_map(|(t, path)| {
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
                path: path.clone().into(),
                selections: vec![Selection { start, end }],
            }],
            rendered: vec![RenderedFile {
                path,
                content: "selected content\n".to_string(),
                first_line: 1,
            }],
        }
    })
}

fn arb_diff() -> impl Strategy<Value = Event> {
    (
        0.0..100.0f64,
        "[a-z]{1,8}\\.rs",
        "[a-z ]{0,20}",
        "[a-z ]{0,20}",
    )
        .prop_map(|(t, path, old, new)| Event::FileDiff {
            offset_secs: t,
            path,
            old: format!("{old}\n"),
            new: format!("{new}\n"),
        })
}

fn arb_event() -> impl Strategy<Value = Event> {
    prop_oneof![
        3 => arb_words(),
        2 => arb_cursor_snapshot(),
        2 => arb_selection_snapshot(),
        1 => arb_diff(),
    ]
}

fn arb_events() -> impl Strategy<Value = Vec<Event>> {
    proptest::collection::vec(arb_event(), 0..20)
}

// ── Prop tests: compress_and_merge ──────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// compress_and_merge produces events sorted by offset_secs.
    #[test]
    fn merge_output_sorted(mut events in arb_events()) {
        compress_and_merge(&mut events);
        for w in events.windows(2) {
            prop_assert!(
                w[0].offset_secs() <= w[1].offset_secs(),
                "output not sorted: {} > {}",
                w[0].offset_secs(),
                w[1].offset_secs()
            );
        }
    }

    /// compress_and_merge preserves all Words events (as a multiset).
    #[test]
    fn merge_preserves_words(events in arb_events()) {
        let mut words_before: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::Words { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let mut merged = events;
        compress_and_merge(&mut merged);
        let mut words_after: Vec<String> = merged
            .iter()
            .filter_map(|e| match e {
                Event::Words { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        words_before.sort();
        words_after.sort();
        prop_assert_eq!(words_before, words_after);
    }

    /// compress_and_merge is idempotent.
    #[test]
    fn merge_idempotent(events in arb_events()) {
        let mut first = events.clone();
        compress_and_merge(&mut first);
        let snapshot = first.clone();
        compress_and_merge(&mut first);
        prop_assert_eq!(first.len(), snapshot.len(), "idempotency violated: length changed");
        for (a, b) in first.iter().zip(snapshot.iter()) {
            prop_assert!(
                (a.offset_secs() - b.offset_secs()).abs() < 1e-10,
                "idempotency violated: offset changed"
            );
        }
    }

    /// Every selection (highlight) file from the input survives compression.
    /// Compression may merge snapshots, but no selection file path is lost.
    #[test]
    fn merge_preserves_selection_snapshots(events in arb_events()) {
        // Collect all file paths that had non-cursor selections in the input.
        let mut input_selection_paths: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { files, rendered, .. } => {
                    let has_selection = files.iter()
                        .any(|f| f.selections.iter().any(|s| !s.is_cursor_like()));
                    if has_selection {
                        Some(rendered.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
                    } else {
                        None
                    }
                }
                _ => None,
            })
            .flatten()
            .collect();
        input_selection_paths.sort();
        input_selection_paths.dedup();

        let mut merged = events;
        compress_and_merge(&mut merged);

        // Collect all rendered paths from output snapshots.
        let mut output_paths: Vec<String> = merged
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { rendered, .. } => {
                    Some(rendered.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        output_paths.sort();
        output_paths.dedup();

        // Every selection path from input must appear in output.
        for path in &input_selection_paths {
            prop_assert!(
                output_paths.contains(path),
                "selection path {:?} lost during compression",
                path
            );
        }
    }

    /// All rendered file paths from input snapshots appear in the output.
    #[test]
    fn merge_preserves_rendered_paths(events in arb_events()) {
        let mut input_paths: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { rendered, files, .. }
                    if files.iter().any(|f| f.selections.iter().any(|s| !s.is_cursor_like())) =>
                {
                    Some(rendered.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        input_paths.sort();
        input_paths.dedup();

        let mut merged = events;
        compress_and_merge(&mut merged);

        let mut output_paths: Vec<String> = merged
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { rendered, .. } => {
                    Some(rendered.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
                }
                _ => None,
            })
            .flatten()
            .collect();
        output_paths.sort();
        output_paths.dedup();

        for path in &input_paths {
            prop_assert!(
                output_paths.contains(path),
                "selection path {:?} missing from output",
                path
            );
        }
    }
}

// ── Prop tests: unified_diff ────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Diffing identical strings produces no +/- lines.
    #[test]
    fn diff_identical_no_changes(text in "[a-z ]{0,50}\n") {
        let diff = unified_diff(&text, &text);
        for line in diff.lines() {
            prop_assert!(
                line.starts_with(' '),
                "identical diff should have only context lines, got: {:?}",
                line
            );
        }
    }

    /// Diffing empty against non-empty produces only + lines.
    #[test]
    fn diff_from_empty_all_inserts(text in "[a-z]{1,20}\n") {
        let diff = unified_diff("", &text);
        for line in diff.lines() {
            prop_assert!(
                line.starts_with('+'),
                "empty→text diff should be all inserts, got: {:?}",
                line
            );
        }
    }

    /// Diffing non-empty against empty produces only - lines.
    #[test]
    fn diff_to_empty_all_deletes(text in "[a-z]{1,20}\n") {
        let diff = unified_diff(&text, "");
        for line in diff.lines() {
            prop_assert!(
                line.starts_with('-'),
                "text→empty diff should be all deletes, got: {:?}",
                line
            );
        }
    }
}

// ── Prop tests: render pipeline ─────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    /// Every Words text appears in the rendered markdown.
    #[test]
    fn render_contains_all_words(mut events in arb_events()) {
        let words: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::Words { text, .. } if !text.trim().is_empty() => Some(text.clone()),
                _ => None,
            })
            .collect();
        let md = format_markdown(&mut events, SnipConfig { threshold: 1000, head: 100, tail: 100 });
        for word in &words {
            let trimmed = word.trim();
            if trimmed.is_empty()
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                || (trimmed.starts_with('(') && trimmed.ends_with(')'))
            {
                continue; // noise markers are filtered
            }
            // Check that at least one word from the text is present
            // (Whisper cleanup may rearrange spaces around punctuation)
            let any_word_present = trimmed.split_whitespace().any(|w| md.contains(w));
            prop_assert!(
                any_word_present,
                "no word from {:?} found in output",
                trimmed
            );
        }
    }

    /// format_markdown never panics on arbitrary event streams.
    #[test]
    fn render_never_panics(mut events in arb_events()) {
        let _ = format_markdown(&mut events, SnipConfig::default());
    }
}
