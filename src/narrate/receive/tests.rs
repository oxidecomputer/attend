use std::path::{Path, PathBuf};

use camino::{Utf8Path, Utf8PathBuf};

use super::*;
use crate::narrate::merge::{Event, RenderedFile};
use crate::state::SessionId;

/// Collecting pending files from a nonexistent session returns empty.
#[test]
fn collect_pending_empty_dir() {
    let sid = SessionId::from("nonexistent-session");
    let files = collect_pending(&sid);
    assert!(files.is_empty());
}

/// An empty file list produces no narration output.
#[test]
fn read_pending_empty() {
    let cwd = Utf8Path::new("/project");
    assert!(read_pending(&[], cwd, &[]).is_none());
}

/// A single JSON file with a Words event renders as prose.
#[test]
fn read_pending_single_json() {
    let dir = tempfile::tempdir().unwrap();
    let events = vec![Event::Words {
        offset_secs: 0.0,
        text: "hello world".to_string(),
    }];
    let path = dir.path().join("2026-02-18T10-00-00Z.json");
    fs::write(&path, serde_json::to_string(&events).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[path], cwd, &[]).unwrap();
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

    let cwd = Utf8Path::new("/project");
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

/// Files under include_dirs pass the cwd filter.
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

    let cwd = Utf8Path::new("/project");
    // Without include_dirs, the file is filtered out
    assert!(read_pending(std::slice::from_ref(&path), cwd, &[]).is_none());

    // With include_dirs, the file passes
    let include = vec![Utf8PathBuf::from("/shared")];
    let result = read_pending(&[path], cwd, &include).unwrap();
    assert!(result.contains("/shared/utils.rs"));
}

/// Words events always pass the cwd filter.
#[test]
fn filter_events_keeps_words() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::Words {
        offset_secs: 0.0,
        text: "hello".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert_eq!(events.len(), 1);
}

/// Diffs for files outside cwd are dropped.
#[test]
fn filter_events_drops_outside_diff() {
    let cwd = Utf8Path::new("/project");
    let mut events = vec![Event::FileDiff {
        offset_secs: 0.0,
        path: "/other/file.rs".to_string(),
        old: "a\n".to_string(),
        new: "b\n".to_string(),
    }];
    filter_events(&mut events, cwd, &[]);
    assert!(events.is_empty());
}

/// Paths are made relative to cwd after filtering.
#[test]
fn relativize_events_strips_prefix() {
    let cwd = Utf8Path::new("/project");
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

/// Multiple JSON files are merged chronologically into one markdown document
/// with prose and fenced code blocks interleaved.
#[test]
fn read_pending_merges_multiple_files() {
    let dir = tempfile::tempdir().unwrap();

    // First file: words + editor snapshot
    let events1 = vec![
        Event::Words {
            offset_secs: 0.0,
            text: "look at this".to_string(),
        },
        Event::EditorSnapshot {
            offset_secs: 1.0,
            files: vec![],
            rendered: vec![RenderedFile {
                path: "/project/src/main.rs".to_string(),
                content: "fn main() {}\n".to_string(),
                first_line: 1,
            }],
        },
    ];
    // Second file: words timestamped after the first file's events.
    let events2 = vec![Event::Words {
        offset_secs: 2.0,
        text: "refactor that".to_string(),
    }];

    let f1 = dir.path().join("2026-02-18T10-00-00Z.json");
    let f2 = dir.path().join("2026-02-18T10-00-01Z.json");
    fs::write(&f1, serde_json::to_string(&events1).unwrap()).unwrap();
    fs::write(&f2, serde_json::to_string(&events2).unwrap()).unwrap();

    let cwd = Utf8Path::new("/project");
    let result = read_pending(&[f1, f2], cwd, &[]).unwrap();
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
        offset_secs: 0.0,
        text: "first dictation".to_string(),
    }];
    let events2 = vec![Event::Words {
        offset_secs: 1.0,
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
    let content = read_pending(&files, cwd, &[]).unwrap();
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
