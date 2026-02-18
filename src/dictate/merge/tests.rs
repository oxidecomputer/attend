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
