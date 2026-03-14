use super::super::*;

/// Convert seconds to a UTC timestamp (for test brevity).
fn ts(secs: f64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds((secs * 1000.0) as i64)
}

/// Words display includes HH:MM:SS timestamp and text preview.
#[test]
fn words_shows_timestamp_and_text() {
    let event = Event::Words {
        timestamp: ts(3661.0), // 01:01:01
        text: "hello world".into(),
    };
    assert_eq!(event.to_string(), r#"Words[01:01:01]: "hello world""#);
}

/// Words display truncates text longer than 40 characters with "...".
#[test]
fn words_truncates_long_text() {
    let long = "a".repeat(50);
    let event = Event::Words {
        timestamp: ts(0.0),
        text: long,
    };
    let display = event.to_string();
    assert!(
        display.ends_with("...\""),
        "expected trailing ...: {display}"
    );
    // 40 chars of 'a' plus "..."
    assert!(
        display.contains(&"a".repeat(40)),
        "expected 40 chars of content"
    );
    assert!(
        !display.contains(&"a".repeat(41)),
        "should not contain 41 chars"
    );
}

/// Words display does not truncate text at exactly 40 characters.
#[test]
fn words_exact_40_no_truncation() {
    let exact = "b".repeat(40);
    let event = Event::Words {
        timestamp: ts(0.0),
        text: exact.clone(),
    };
    let display = event.to_string();
    assert!(
        !display.contains("..."),
        "40-char text should not be truncated: {display}"
    );
    assert!(display.contains(&exact));
}

/// EditorSnapshot display shows region count.
#[test]
fn editor_snapshot_shows_region_count() {
    let event = Event::EditorSnapshot {
        timestamp: ts(7200.0), // 02:00:00
        last_seen: ts(7200.0),
        files: vec![],
        regions: vec![
            CapturedRegion {
                path: "a.rs".into(),
                content: "fn a() {}".into(),
                first_line: 1,
                selections: vec![],
                language: None,
            },
            CapturedRegion {
                path: "b.rs".into(),
                content: "fn b() {}".into(),
                first_line: 1,
                selections: vec![],
                language: None,
            },
        ],
    };
    assert_eq!(event.to_string(), "EditorSnapshot[02:00:00]: 2 region(s)");
}

/// FileDiff display shows the file path.
#[test]
fn file_diff_shows_path() {
    let event = Event::FileDiff {
        timestamp: ts(0.0),
        path: "src/main.rs".into(),
        old: String::new(),
        new: String::new(),
    };
    assert_eq!(event.to_string(), "FileDiff[00:00:00]: src/main.rs");
}

/// ExternalSelection display uses full variant name and shows app + window title.
#[test]
fn external_selection_shows_app_and_window() {
    let event = Event::ExternalSelection {
        timestamp: ts(3600.0), // 01:00:00
        last_seen: ts(3600.0),
        app: "Firefox".into(),
        window_title: "Rust docs".into(),
        text: "some selected text".into(),
    };
    assert_eq!(
        event.to_string(),
        "ExternalSelection[01:00:00]: Firefox - Rust docs"
    );
}

/// BrowserSelection display shows title and URL.
#[test]
fn browser_selection_shows_title_and_url() {
    let event = Event::BrowserSelection {
        timestamp: ts(0.0),
        last_seen: ts(0.0),
        url: "https://doc.rust-lang.org".into(),
        title: "std docs".into(),
        text: "selected".into(),
        plain_text: "selected".into(),
    };
    assert_eq!(
        event.to_string(),
        "BrowserSelection[00:00:00]: std docs (https://doc.rust-lang.org)"
    );
}

/// ShellCommand display truncates long commands and shows exit status.
#[test]
fn shell_command_with_exit_status() {
    let event = Event::ShellCommand {
        timestamp: ts(0.0),
        shell: ShellKind::Fish,
        command: "cargo test".into(),
        cwd: "/home/user".into(),
        exit_status: Some(0),
        duration_secs: Some(1.5),
    };
    assert_eq!(
        event.to_string(),
        r#"ShellCommand[00:00:00]: "cargo test" (exit 0)"#
    );
}

/// ShellCommand display shows "(running)" for preexec-only events without exit status.
#[test]
fn shell_command_running() {
    let event = Event::ShellCommand {
        timestamp: ts(0.0),
        shell: ShellKind::Zsh,
        command: "make".into(),
        cwd: "/tmp".into(),
        exit_status: None,
        duration_secs: None,
    };
    assert_eq!(
        event.to_string(),
        r#"ShellCommand[00:00:00]: "make" (running)"#
    );
}

/// ShellCommand display truncates commands longer than 40 characters.
#[test]
fn shell_command_truncates_long_command() {
    let long_cmd = "x".repeat(50);
    let event = Event::ShellCommand {
        timestamp: ts(0.0),
        shell: ShellKind::Fish,
        command: long_cmd,
        cwd: "/tmp".into(),
        exit_status: Some(1),
        duration_secs: Some(0.1),
    };
    let display = event.to_string();
    assert!(display.contains("..."), "long command should be truncated");
    assert!(display.contains(&"x".repeat(40)));
    assert!(!display.contains(&"x".repeat(41)));
}

/// ClipboardSelection text variant shows content type and text preview.
#[test]
fn clipboard_text_shows_content_type() {
    let event = Event::ClipboardSelection {
        timestamp: ts(0.0),
        content: ClipboardContent::Text {
            text: "copied text".into(),
        },
    };
    assert_eq!(
        event.to_string(),
        r#"ClipboardSelection[00:00:00]: text "copied text""#
    );
}

/// ClipboardSelection image variant shows content type and image path.
#[test]
fn clipboard_image_shows_content_type() {
    let event = Event::ClipboardSelection {
        timestamp: ts(0.0),
        content: ClipboardContent::Image {
            path: "/tmp/clip.png".into(),
        },
    };
    assert_eq!(
        event.to_string(),
        "ClipboardSelection[00:00:00]: image /tmp/clip.png"
    );
}

/// ClipboardSelection text variant truncates long text.
#[test]
fn clipboard_text_truncates() {
    let long_text = "z".repeat(45);
    let event = Event::ClipboardSelection {
        timestamp: ts(0.0),
        content: ClipboardContent::Text { text: long_text },
    };
    let display = event.to_string();
    assert!(
        display.contains("..."),
        "long clipboard text should be truncated"
    );
    assert!(display.contains(&"z".repeat(40)));
    assert!(!display.contains(&"z".repeat(41)));
}

/// Redacted display includes timestamp and the kind of event that was redacted.
#[test]
fn redacted_shows_kind() {
    let event = Event::Redacted {
        timestamp: ts(0.0),
        kind: RedactedKind::FileDiff,
        keys: vec!["src/lib.rs".into()],
    };
    assert_eq!(event.to_string(), "Redacted[00:00:00]: FileDiff");
}

/// All variants include an HH:MM:SS timestamp in their display output.
#[test]
fn all_variants_include_timestamp() {
    let t = ts(45296.0); // 12:34:56
    let events = vec![
        Event::Words {
            timestamp: t,
            text: "hi".into(),
        },
        Event::EditorSnapshot {
            timestamp: t,
            last_seen: t,
            files: vec![],
            regions: vec![],
        },
        Event::FileDiff {
            timestamp: t,
            path: "f.rs".into(),
            old: String::new(),
            new: String::new(),
        },
        Event::ExternalSelection {
            timestamp: t,
            last_seen: t,
            app: "app".into(),
            window_title: "win".into(),
            text: "sel".into(),
        },
        Event::BrowserSelection {
            timestamp: t,
            last_seen: t,
            url: "https://x".into(),
            title: "t".into(),
            text: "s".into(),
            plain_text: "s".into(),
        },
        Event::ShellCommand {
            timestamp: t,
            shell: ShellKind::Fish,
            command: "ls".into(),
            cwd: "/".into(),
            exit_status: Some(0),
            duration_secs: Some(0.0),
        },
        Event::ClipboardSelection {
            timestamp: t,
            content: ClipboardContent::Text {
                text: "clip".into(),
            },
        },
        Event::Redacted {
            timestamp: t,
            kind: RedactedKind::ShellCommand,
            keys: vec![],
        },
    ];
    for event in &events {
        let display = event.to_string();
        assert!(
            display.contains("12:34:56"),
            "missing timestamp in: {display}"
        );
    }
}
