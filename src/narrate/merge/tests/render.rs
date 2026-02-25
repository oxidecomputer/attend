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

`src/main.rs:1`:
```
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

`src/lib.rs`:
```diff
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
        "\n`src/lib.rs:42`:\n```\npub fn add(a: i32, b: i32) -> i32 {\n    a + b\n}\n```\n";
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
    assert!(md.contains("`src/a.py:1`:\n```"));
    assert!(md.contains("`src/b.js:10`:\n```"));
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
    assert!(md.contains("`foo.rs`:\n```diff\n+new line\n```\n"));
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

// ── ShellCommand rendering ─────────────────────────────────────────────────

/// Postexec with non-zero exit renders exit status and duration.
#[test]
fn shell_command_postexec_failure() {
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(1.0),
        shell: "fish".to_string(),
        command: "cargo test".to_string(),
        cwd: ".".to_string(),
        exit_status: Some(1),
        duration_secs: Some(3.2),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("```fish\n"), "should have fish language tag");
    assert!(
        md.contains("$ cargo test"),
        "should have $ prefixed command"
    );
    assert!(
        md.contains("# exit 1, 3.2s"),
        "should have exit+duration comment: {md:?}"
    );
    assert!(md.contains("```\n"), "should have closing fence");
}

/// Postexec with exit 0 and short duration omits the comment.
#[test]
fn shell_command_postexec_fast_success() {
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(1.0),
        shell: "fish".to_string(),
        command: "cargo fmt".to_string(),
        cwd: ".".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.3),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        md.contains("$ cargo fmt\n```"),
        "no trailing comment: {md:?}"
    );
    assert!(!md.contains("# exit"), "no exit comment for fast success");
}

/// Postexec with exit 0 but long duration shows duration.
#[test]
fn shell_command_postexec_slow_success() {
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(1.0),
        shell: "fish".to_string(),
        command: "cargo build".to_string(),
        cwd: ".".to_string(),
        exit_status: Some(0),
        duration_secs: Some(45.0),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        md.contains("# exit 0, 45.0s"),
        "should show duration for slow success: {md:?}"
    );
}

/// Preexec (no exit status) renders without comment.
#[test]
fn shell_command_preexec() {
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(1.0),
        shell: "zsh".to_string(),
        command: "cargo test".to_string(),
        cwd: ".".to_string(),
        exit_status: None,
        duration_secs: None,
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("```zsh\n"), "should have zsh language tag");
    assert!(md.contains("$ cargo test\n```"), "no trailing comment");
}

/// Shell command with non-project cwd shows directory comment.
#[test]
fn shell_command_with_cwd() {
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(1.0),
        shell: "fish".to_string(),
        command: "ls".to_string(),
        cwd: "subdir/nested".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.1),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        md.contains("In `subdir/nested/`:\n"),
        "should show cwd comment: {md:?}"
    );
}

/// Shell command at project root (cwd = ".") omits directory comment.
#[test]
fn shell_command_project_root_no_cwd() {
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(1.0),
        shell: "fish".to_string(),
        command: "cargo test".to_string(),
        cwd: ".".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.1),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(!md.contains("In `"), "no cwd label at project root");
}

/// Shell command interleaved with speech renders in chronological order.
#[test]
fn shell_command_interleaved_with_speech() {
    let mut events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "let me run the tests".to_string(),
        },
        Event::ShellCommand {
            timestamp: ts(1.0),
            shell: "fish".to_string(),
            command: "cargo test".to_string(),
            cwd: ".".to_string(),
            exit_status: Some(1),
            duration_secs: Some(3.2),
        },
        Event::Words {
            timestamp: ts(5.0),
            text: "they failed".to_string(),
        },
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    let test_pos = md.find("cargo test").unwrap();
    let let_pos = md.find("let me run").unwrap();
    let fail_pos = md.find("they failed").unwrap();
    assert!(let_pos < test_pos, "speech before command");
    assert!(test_pos < fail_pos, "command before subsequent speech");
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
        md.contains("`src/main.rs:1`:\n```\n"),
        "fence should be bare when no language: {md:?}"
    );
    assert!(
        !md.contains("```rust"),
        "no language tag should appear: {md:?}"
    );
}
