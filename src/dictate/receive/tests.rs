use std::path::Path;

use super::*;
use crate::dictate::merge::{Event, RenderedFile};

#[test]
fn collect_pending_empty_dir() {
    let files = collect_pending("nonexistent-session");
    assert!(files.is_empty());
}

#[test]
fn read_pending_empty() {
    let cwd = Path::new("/project");
    assert!(read_pending(&[], cwd, &[]).is_none());
}

#[test]
fn read_pending_single_json() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![Event::Words {
        offset_secs: 0.0,
        text: "hello world".to_string(),
    }];
    let path = dir.path().join("2026-02-18T10-00-00Z.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Path::new("/project");
    let result = read_pending(&[path], cwd, &[]).unwrap();
    assert!(result.contains("hello world"));
    assert!(result.starts_with("<dictation>"));
    assert!(result.ends_with("</dictation>"));
}

#[test]
fn read_pending_filters_by_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![
        Event::Words {
            offset_secs: 0.0,
            text: "look at this".to_string(),
        },
        Event::EditorSnapshot {
            offset_secs: 1.0,
            files: vec![],
            rendered: vec![
                RenderedFile {
                    path: "/project/src/main.rs".to_string(),
                    content: "fn main() {}\n".to_string(),
                    first_line: 1,
                },
                RenderedFile {
                    path: "/other/lib.rs".to_string(),
                    content: "fn other() {}\n".to_string(),
                    first_line: 1,
                },
            ],
        },
    ];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Path::new("/project");
    let result = read_pending(&[path], cwd, &[]).unwrap();
    assert!(
        result.contains("src/main.rs"),
        "project file should be included"
    );
    assert!(
        !result.contains("/other/lib.rs"),
        "outside file should be filtered out"
    );
}

#[test]
fn read_pending_includes_extra_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![Event::EditorSnapshot {
        offset_secs: 0.0,
        files: vec![],
        rendered: vec![RenderedFile {
            path: "/shared/utils.rs".to_string(),
            content: "fn shared() {}\n".to_string(),
            first_line: 1,
        }],
    }];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Path::new("/project");
    // Without include_dirs, the file is filtered out
    assert!(read_pending(&[path.clone()], cwd, &[]).is_none());

    // With include_dirs, the file passes
    let include = vec![PathBuf::from("/shared")];
    let result = read_pending(&[path], cwd, &include).unwrap();
    assert!(result.contains("/shared/utils.rs"));
}

#[test]
fn filter_events_keeps_words() {
    let cwd = Path::new("/project");
    let mut events = vec![Event::Words {
        offset_secs: 0.0,
        text: "hello".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1);
}

#[test]
fn filter_events_drops_outside_diff() {
    let cwd = Path::new("/project");
    let mut events = vec![Event::FileDiff {
        offset_secs: 0.0,
        path: "/other/file.rs".to_string(),
        old: "a\n".to_string(),
        new: "b\n".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert!(events.is_empty());
}

#[test]
fn relativize_events_strips_prefix() {
    let cwd = Path::new("/project");
    let mut events = vec![
        Event::EditorSnapshot {
            offset_secs: 0.0,
            files: vec![],
            rendered: vec![RenderedFile {
                path: "/project/src/lib.rs".to_string(),
                content: "code\n".to_string(),
                first_line: 1,
            }],
        },
        Event::FileDiff {
            offset_secs: 1.0,
            path: "/project/src/main.rs".to_string(),
            old: "a\n".to_string(),
            new: "b\n".to_string(),
        },
    ];
    relativize_events(&mut events, cwd);

    if let Event::EditorSnapshot { rendered, .. } = &events[0] {
        assert_eq!(rendered[0].path, "src/lib.rs");
    } else {
        panic!("expected EditorSnapshot");
    }

    if let Event::FileDiff { path, .. } = &events[1] {
        assert_eq!(path, "src/main.rs");
    } else {
        panic!("expected FileDiff");
    }
}

#[test]
fn dictation_tags_wrapping() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![Event::Words {
        offset_secs: 0.0,
        text: "test message".to_string(),
    }];
    let path = dir.path().join("test.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Path::new("/project");
    let result = read_pending(&[path], cwd, &[]).unwrap();
    assert!(result.starts_with("<dictation>\n"));
    assert!(result.ends_with("\n</dictation>"));
    assert!(result.contains("test message"));
}

#[test]
fn lock_guard_cleanup() {
    let dir = tempfile::tempdir().unwrap();
    let lock_path = dir.path().join("test.lock");

    {
        let _guard = try_lock(&lock_path).expect("should acquire lock");
        assert!(lock_path.exists());

        // Second attempt should fail
        assert!(try_lock(&lock_path).is_none());
    }

    // After drop, lock should be removed
    assert!(!lock_path.exists());
}
