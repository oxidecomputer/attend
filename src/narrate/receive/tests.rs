use std::fs;
use std::path::{Path, PathBuf};

use camino::{Utf8Path, Utf8PathBuf};

use super::filter::{filter_events, relativize_events};
use super::listen::try_lock;
use super::pending::{collect_pending, collect_pending_dir};
use super::read_pending;
use crate::narrate::merge::{CapturedRegion, Event};
use crate::state::SessionId;

/// Convert seconds to a UTC timestamp (for test brevity).
fn ts(secs: f64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds((secs * 1000.0) as i64)
}

/// Collecting pending files from an empty cache tree returns empty.
///
/// Uses a CacheDirGuard so the test doesn't observe real `_local/` files
/// left by the daemon on the developer's machine.
#[test]
fn collect_pending_empty_dir() {
    let _g = crate::state::CacheDirGuard::new();

    let sid = SessionId::from("nonexistent-session");
    let files = collect_pending(&sid);
    assert!(files.is_empty());
}

/// An empty file list produces no narration output.
#[test]
fn read_pending_empty() {
    let cwd = Utf8Path::new("/project");
    assert!(read_pending(&[], Some(cwd), &[]).is_none());
}

/// A single JSON file with a Words event renders as prose.
#[test]
fn read_pending_single_json() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![Event::Words {
        timestamp: ts(0.0),
        text: "hello world".to_string(),
    }];
    let path = dir.path().join("2026-02-18T10-00-00Z.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[path], Some(cwd), &[]).unwrap();
    assert!(result.contains("hello world"));
    // read_pending returns raw markdown; <narration> tags are applied at render time.
    assert!(!result.contains("<narration>"));
}

/// Editor snapshots for files outside the cwd are filtered out.
#[test]
fn read_pending_filters_by_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "look at this".to_string(),
        },
        Event::EditorSnapshot {
            timestamp: ts(1.0),
            last_seen: ts(1.0),
            files: vec![],
            regions: vec![
                CapturedRegion {
                    path: "/project/src/main.rs".to_string(),
                    content: "fn main() {}\n".to_string(),
                    first_line: 1,
                    selections: vec![],
                    language: None,
                },
                CapturedRegion {
                    path: "/other/lib.rs".to_string(),
                    content: "fn other() {}\n".to_string(),
                    first_line: 1,
                    selections: vec![],
                    language: None,
                },
            ],
        },
    ];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[path], Some(cwd), &[]).unwrap();
    assert!(
        result.contains("src/main.rs"),
        "project file should be included"
    );
    assert!(
        !result.contains("/other/lib.rs"),
        "outside file should be filtered out"
    );
}

/// Files under include_dirs pass the cwd filter.
#[test]
fn read_pending_includes_extra_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "look at shared".to_string(),
        },
        Event::EditorSnapshot {
            timestamp: ts(1.0),
            last_seen: ts(1.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "/shared/utils.rs".to_string(),
                content: "fn shared() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
    ];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    // Without include_dirs, the snapshot is filtered out (only words remain).
    let result = read_pending(std::slice::from_ref(&path), Some(cwd), &[]).unwrap();
    assert!(!result.contains("/shared/utils.rs"));

    // With include_dirs, the snapshot passes.
    let include = vec![Utf8PathBuf::from("/shared")];
    let result = read_pending(&[path], Some(cwd), &include).unwrap();
    assert!(result.contains("/shared/utils.rs"));
}

/// Words events always pass the cwd filter.
#[test]
fn filter_events_keeps_words() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::Words {
        timestamp: ts(0.0),
        text: "hello".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1);
}

/// Diffs for files outside cwd are replaced with a Redacted marker.
#[test]
fn filter_events_redacts_outside_diff() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::FileDiff {
        timestamp: ts(0.0),
        path: "/other/file.rs".to_string(),
        old: "a\n".to_string(),
        new: "b\n".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1, "should have one Redacted event");
    assert!(
        matches!(
            &events[0],
            Event::Redacted {
                kind: crate::narrate::merge::RedactedKind::FileDiff,
                keys,
                ..
            } if keys == &["/other/file.rs"]
        ),
        "should be a Redacted FileDiff with the filtered path"
    );
}

/// External selections pass through the filter unconditionally (no file paths to check).
#[test]
fn filter_events_keeps_ext_selection() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ExternalSelection {
        timestamp: ts(0.0),
        last_seen: ts(0.0),
        app: "iTerm2".to_string(),
        window_title: "~/other-project".to_string(),
        text: "error message".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1, "external selection should pass through");
}

/// Browser selections pass through the filter unconditionally (no file paths to check).
#[test]
fn filter_events_keeps_browser_selection() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::BrowserSelection {
        timestamp: ts(0.0),
        last_seen: ts(0.0),
        url: "https://example.com".to_string(),
        title: "Example Page".to_string(),
        text: "some text".to_string(),
        plain_text: "some text".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1, "browser selection should pass through");
}

/// Paths are made relative to cwd after filtering.
#[test]
fn relativize_events_strips_prefix() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![
        Event::EditorSnapshot {
            timestamp: ts(0.0),
            last_seen: ts(0.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "/project/src/lib.rs".to_string(),
                content: "code\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
        Event::FileDiff {
            timestamp: ts(1.0),
            path: "/project/src/main.rs".to_string(),
            old: "a\n".to_string(),
            new: "b\n".to_string(),
        },
    ];
    relativize_events(&mut events, cwd);

    if let Event::EditorSnapshot { regions, .. } = &events[0] {
        assert_eq!(regions[0].path, "src/lib.rs");
    } else {
        panic!("expected EditorSnapshot");
    }

    if let Event::FileDiff { path, .. } = &events[1] {
        assert_eq!(path, "src/main.rs");
    } else {
        panic!("expected FileDiff");
    }
}

/// Multiple JSON files are merged chronologically into one markdown document
/// with prose and fenced code blocks interleaved.
#[test]
fn read_pending_merges_multiple_files() {
    let dir = tempfile::tempdir().unwrap();

    // First file: words + editor snapshot
    let events1 = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "look at this".to_string(),
        },
        Event::EditorSnapshot {
            timestamp: ts(1.0),
            last_seen: ts(1.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "/project/src/main.rs".to_string(),
                content: "fn main() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
    ];
    // Second file: words timestamped after the first file's events.
    let events2 = vec![Event::Words {
        timestamp: ts(2.0),
        text: "refactor that".to_string(),
    }];

    let f1 = dir.path().join("2026-02-18T10-00-00Z.json");
    let f2 = dir.path().join("2026-02-18T10-00-01Z.json");
    fs::write(&f1, serde_json::to_string(&events1).unwrap()).unwrap();
    fs::write(&f2, serde_json::to_string(&events2).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[f1, f2], Some(cwd), &[]).unwrap();
    // Prose from both files appears.
    assert!(result.contains("look at this"));
    assert!(result.contains("refactor that"));
    // Code block from the snapshot appears.
    assert!(result.contains("```"));
    assert!(result.contains("fn main()"));
    // Path is relativized.
    assert!(result.contains("src/main.rs"));
}

/// Lock guard removes the lock file on drop and prevents double-acquisition.
#[test]
fn lock_guard_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = Utf8PathBuf::try_from(dir.path().join("test.lock")).unwrap();

    {
        let _guard = try_lock(&lock_path).expect("should acquire lock");
        assert!(lock_path.exists());

        // Second attempt should fail
        assert!(try_lock(&lock_path).is_none());
    }

    // After drop, lock should be removed
    assert!(!lock_path.exists());
}

// -- Integration: collect -> read -> archive cycle --

/// Full cycle: collect pending files, read into markdown, archive moves files.
#[test]
fn collect_read_archive_round_trip() {
    let base = tempfile::tempdir().unwrap();
    let session_id = "int-test-session";

    // Set up a pending directory with two narration files.
    let pending = base.path().join("pending").join(session_id);
    fs::create_dir_all(&pending).unwrap();

    let events1 = vec![Event::Words {
        timestamp: ts(0.0),
        text: "first dictation".to_string(),
    }];
    let events2 = vec![Event::Words {
        timestamp: ts(1.0),
        text: "second dictation".to_string(),
    }];

    let f1 = pending.join("2026-02-18T10-00-00Z.json");
    let f2 = pending.join("2026-02-18T10-00-01Z.json");
    fs::write(&f1, serde_json::to_string(&events1).unwrap()).unwrap();
    fs::write(&f2, serde_json::to_string(&events2).unwrap()).unwrap();

    // Collect should find both files in order.
    let files = collect_pending_from(&pending);
    assert_eq!(files.len(), 2);

    // Read should merge into a single narration block.
    let cwd = Utf8Path::new("/project");
    let content = read_pending(&files, Some(cwd), &[]).unwrap();
    assert!(content.contains("first dictation"));
    assert!(content.contains("second dictation"));

    // Archive should move files out of pending.
    let archive = base.path().join("archive").join(session_id);
    fs::create_dir_all(&archive).unwrap();
    for path in &files {
        if let Some(filename) = path.file_name() {
            let _ = fs::rename(path, archive.join(filename));
        }
    }
    assert!(!f1.exists());
    assert!(!f2.exists());
    assert!(archive.join("2026-02-18T10-00-00Z.json").exists());
    assert!(archive.join("2026-02-18T10-00-01Z.json").exists());
}

/// Helper: collect pending files from an arbitrary directory (not cache_dir).
fn collect_pending_from(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

// -- No-session (_local) support tests --

/// Shell commands with cwd inside the project pass the filter.
#[test]
fn filter_events_keeps_shell_command_inside_cwd() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(0.0),
        shell: "fish".to_string(),
        command: "cargo test".to_string(),
        cwd: "/project/src".to_string(),
        exit_status: Some(0),
        duration_secs: Some(2.5),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1, "shell command inside cwd should pass");
}

/// Shell commands with cwd outside the project are replaced with a Redacted marker.
#[test]
fn filter_events_redacts_shell_command_outside_cwd() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(0.0),
        shell: "zsh".to_string(),
        command: "ls".to_string(),
        cwd: "/other-project".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.1),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1, "should have one Redacted event");
    assert!(
        matches!(
            &events[0],
            Event::Redacted {
                kind: crate::narrate::merge::RedactedKind::ShellCommand,
                keys,
                ..
            } if keys == &["ls"]
        ),
        "should be a Redacted ShellCommand with the command text"
    );
}

/// Shell command cwd is relativized to the project root.
#[test]
fn relativize_events_strips_shell_command_cwd() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(0.0),
        shell: "fish".to_string(),
        command: "cargo build".to_string(),
        cwd: "/project/src/lib".to_string(),
        exit_status: Some(0),
        duration_secs: Some(5.0),
    }];
    relativize_events(&mut events, cwd);

    if let Event::ShellCommand { cwd: cmd_cwd, .. } = &events[0] {
        assert_eq!(cmd_cwd, "src/lib", "cwd should be relativized");
    } else {
        panic!("expected ShellCommand");
    }
}

/// Shell command at project root gets cwd relativized to ".".
#[test]
fn relativize_events_shell_command_at_root() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(0.0),
        shell: "fish".to_string(),
        command: "cargo fmt".to_string(),
        cwd: "/project".to_string(),
        exit_status: Some(0),
        duration_secs: Some(0.3),
    }];
    relativize_events(&mut events, cwd);

    if let Event::ShellCommand { cwd: cmd_cwd, .. } = &events[0] {
        assert!(
            cmd_cwd.is_empty(),
            "project root should relativize to empty string"
        );
    } else {
        panic!("expected ShellCommand");
    }
}

/// Shell command in include_dirs passes the filter.
#[test]
fn filter_events_keeps_shell_command_in_include_dirs() {
    let cwd = Utf8Path::new("/project");
    let include = vec![Utf8PathBuf::from("/shared")];
    let mut events = vec![Event::ShellCommand {
        timestamp: ts(0.0),
        shell: "zsh".to_string(),
        command: "make".to_string(),
        cwd: "/shared/build".to_string(),
        exit_status: Some(0),
        duration_secs: Some(1.0),
    }];
    filter_events(&mut events, cwd, &include);
    assert_eq!(events.len(), 1, "shell command in include_dirs should pass");
}

/// End-to-end: shell commands in pending JSON survive the full read_pending pipeline.
#[test]
fn read_pending_renders_shell_command() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "then I ran".to_string(),
        },
        Event::ShellCommand {
            timestamp: ts(1.0),
            shell: "fish".to_string(),
            command: "cargo test --lib".to_string(),
            cwd: "/project".to_string(),
            exit_status: Some(1),
            duration_secs: Some(3.2),
        },
    ];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[path], Some(cwd), &[]).unwrap();
    assert!(result.contains("then I ran"), "prose should be included");
    assert!(
        result.contains("cargo test --lib"),
        "command should be rendered"
    );
    assert!(result.contains("```fish"), "should have fish code fence");
    assert!(result.contains("exit 1"), "should show non-zero exit");
}

/// `collect_pending_dir` returns files from a single directory, sorted.
#[test]
fn collect_pending_dir_returns_sorted_json() {
    let dir = tempfile::tempdir().unwrap();
    let utf8 = Utf8Path::from_path(dir.path()).unwrap();
    fs::write(dir.path().join("b.json"), "[]").unwrap();
    fs::write(dir.path().join("a.json"), "[]").unwrap();
    fs::write(dir.path().join("c.txt"), "ignored").unwrap();

    let files = collect_pending_dir(utf8);
    assert_eq!(files.len(), 2, "should find only .json files");
    let names: Vec<_> = files.iter().map(|p| p.file_name().unwrap()).collect();
    assert_eq!(names[0], "a.json");
    assert_eq!(names[1], "b.json");
}

/// ClipboardSelection passes through the filter regardless of cwd scope.
#[test]
fn clipboard_passes_through_filter() {
    use crate::narrate::merge::ClipboardContent;

    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ClipboardSelection {
        timestamp: ts(0.0),
        content: ClipboardContent::Text {
            text: "copied text".to_string(),
        },
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(
        events.len(),
        1,
        "clipboard selection should pass through filter"
    );
    assert!(matches!(events[0], Event::ClipboardSelection { .. }));
}

/// ClipboardSelection (including image path) is untouched by relativization.
#[test]
fn clipboard_not_relativized() {
    use crate::narrate::merge::ClipboardContent;

    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::ClipboardSelection {
        timestamp: ts(0.0),
        content: ClipboardContent::Image {
            path: "/Users/oxide/.cache/attend/clipboard-staging/12345.png".to_string(),
        },
    }];
    relativize_events(&mut events, cwd);
    match &events[0] {
        Event::ClipboardSelection {
            content: ClipboardContent::Image { path },
            ..
        } => {
            assert_eq!(
                path, "/Users/oxide/.cache/attend/clipboard-staging/12345.png",
                "clipboard image path should not be relativized"
            );
        }
        other => panic!("expected ClipboardSelection, got {other:?}"),
    }
}

/// `collect_pending_dir` returns empty for a nonexistent directory.
#[test]
fn collect_pending_dir_missing_dir() {
    let utf8 = Utf8Path::new("/tmp/nonexistent-attend-test-dir");
    let files = collect_pending_dir(utf8);
    assert!(files.is_empty());
}

/// The receive pipeline merges _local files with session files.
///
/// Simulates: daemon wrote to `_local` (no session), then a session starts
/// and `collect_pending` picks up both session and _local files.
#[test]
fn collect_pending_merges_session_and_local() {
    let base = tempfile::tempdir().unwrap();
    let session_id = "merge-test-session";

    // Session pending dir
    let session_dir = base.path().join("pending").join(session_id);
    fs::create_dir_all(&session_dir).unwrap();
    let events_session = vec![Event::Words {
        timestamp: ts(1.0),
        text: "session narration".to_string(),
    }];
    fs::write(
        session_dir.join("2026-02-18T10-00-01Z.json"),
        serde_json::to_string(&events_session).unwrap(),
    )
    .unwrap();

    // _local pending dir
    let local_dir = base.path().join("pending").join("_local");
    fs::create_dir_all(&local_dir).unwrap();
    let events_local = vec![Event::Words {
        timestamp: ts(0.0),
        text: "local narration".to_string(),
    }];
    fs::write(
        local_dir.join("2026-02-18T10-00-00Z.json"),
        serde_json::to_string(&events_local).unwrap(),
    )
    .unwrap();

    // Use collect_pending_dir on both dirs (simulating what collect_pending does).
    let session_utf8 = Utf8Path::from_path(&session_dir).unwrap();
    let local_utf8 = Utf8Path::from_path(&local_dir).unwrap();
    let mut files = collect_pending_dir(session_utf8);
    files.extend(collect_pending_dir(local_utf8));
    files.sort();

    assert_eq!(files.len(), 2);

    // Read merges both files into one markdown document.
    let cwd = Utf8Path::new("/project");
    let content = read_pending(&files, Some(cwd), &[]).unwrap();
    assert!(
        content.contains("local narration"),
        "_local narration should be included"
    );
    assert!(
        content.contains("session narration"),
        "session narration should be included"
    );
}

// -- Redaction marker tests --

/// Partial EditorSnapshot filtering: some regions survive, dropped regions
/// produce a Redacted marker with correct file paths.
#[test]
fn filter_events_partial_editor_snapshot_redaction() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::EditorSnapshot {
        timestamp: ts(0.0),
        last_seen: ts(0.0),
        files: vec![],
        regions: vec![
            CapturedRegion {
                path: "/project/src/main.rs".to_string(),
                content: "fn main() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            },
            CapturedRegion {
                path: "/other/lib.rs".to_string(),
                content: "fn other() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            },
            CapturedRegion {
                path: "/elsewhere/util.rs".to_string(),
                content: "fn util() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            },
        ],
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 2, "surviving snapshot + redacted marker");
    assert!(
        matches!(&events[0], Event::EditorSnapshot { regions, .. } if regions.len() == 1),
        "one region should survive"
    );
    assert!(
        matches!(
            &events[1],
            Event::Redacted {
                kind: crate::narrate::merge::RedactedKind::EditorSnapshot,
                keys,
                ..
            } if keys.len() == 2
        ),
        "two distinct files should be redacted"
    );
}

/// Adjacent Redacted events of the same kind are collapsed with key dedup.
#[test]
fn collapse_redacted_merges_adjacent_same_kind() {
    use crate::narrate::merge::RedactedKind;

    let cwd = Utf8Path::new("/project");
    let mut events = vec![
        Event::FileDiff {
            timestamp: ts(0.0),
            path: "/other/a.rs".to_string(),
            old: "a".to_string(),
            new: "b".to_string(),
        },
        Event::FileDiff {
            timestamp: ts(1.0),
            path: "/other/a.rs".to_string(),
            old: "b".to_string(),
            new: "c".to_string(),
        },
        Event::FileDiff {
            timestamp: ts(2.0),
            path: "/other/b.rs".to_string(),
            old: "x".to_string(),
            new: "y".to_string(),
        },
    ];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(
        events.len(),
        1,
        "three diffs should collapse into one Redacted"
    );
    match &events[0] {
        Event::Redacted { kind, keys, .. } => {
            assert_eq!(*kind, RedactedKind::FileDiff);
            assert_eq!(keys.len(), 2, "two distinct file paths after dedup");
        }
        other => panic!("expected Redacted, got {other:?}"),
    }
}

/// Interleaved Redacted kinds in a run are grouped and reordered by kind.
#[test]
fn collapse_redacted_reorders_interleaved_kinds() {
    use crate::narrate::merge::RedactedKind;

    let cwd = Utf8Path::new("/project");
    // Pattern: file, command, file → should collapse to 2 files + 1 command
    let mut events = vec![
        Event::EditorSnapshot {
            timestamp: ts(0.0),
            last_seen: ts(0.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "/other/a.rs".to_string(),
                content: "fn a() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
        Event::ShellCommand {
            timestamp: ts(1.0),
            shell: "fish".to_string(),
            command: "ls".to_string(),
            cwd: "/other".to_string(),
            exit_status: Some(0),
            duration_secs: Some(0.1),
        },
        Event::EditorSnapshot {
            timestamp: ts(2.0),
            last_seen: ts(2.0),
            files: vec![],
            regions: vec![CapturedRegion {
                path: "/other/b.rs".to_string(),
                content: "fn b() {}\n".to_string(),
                first_line: 1,
                selections: vec![],
                language: None,
            }],
        },
    ];
    filter_events(&mut events, cwd, &[]);
    // Run of 3 Redacted events should collapse to 2: EditorSnapshot(2 files) + ShellCommand(1)
    assert_eq!(
        events.len(),
        2,
        "interleaved kinds should reorder into 2 groups"
    );
    // BTreeMap ordering: EditorSnapshot < ShellCommand
    assert!(
        matches!(
            &events[0],
            Event::Redacted { kind: RedactedKind::EditorSnapshot, keys, .. } if keys.len() == 2
        ),
        "first should be EditorSnapshot with 2 files"
    );
    assert!(
        matches!(
            &events[1],
            Event::Redacted { kind: RedactedKind::ShellCommand, keys, .. } if keys.len() == 1
        ),
        "second should be ShellCommand with 1 command"
    );
}

/// Redaction-only pending files yield None from read_pending (not worth delivering).
#[test]
fn read_pending_redaction_only_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![Event::FileDiff {
        timestamp: ts(0.0),
        path: "/other/file.rs".to_string(),
        old: "a\n".to_string(),
        new: "b\n".to_string(),
    }];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    assert!(
        read_pending(&[path], Some(cwd), &[]).is_none(),
        "redaction-only content should not be delivered"
    );
}

/// Redaction markers appear inline when mixed with deliverable content.
#[test]
fn read_pending_renders_redaction_marker() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![
        Event::Words {
            timestamp: ts(0.0),
            text: "look at this".to_string(),
        },
        Event::FileDiff {
            timestamp: ts(1.0),
            path: "/other/file.rs".to_string(),
            old: "a\n".to_string(),
            new: "b\n".to_string(),
        },
    ];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[path], Some(cwd), &[]).unwrap();
    assert!(result.contains("look at this"), "prose should be present");
    assert!(
        result.contains("\u{2702} edit"),
        "redaction marker should be present: got {result:?}"
    );
}
