use super::super::*;

use crate::narrate::render::{SnipConfig, format_markdown};
use crate::state::{Col, FileEntry, Line, Position, Selection};

/// Helper: cursor-only snapshot (all selections are cursor-like).
fn cursor_snap(t: f64, path: &str) -> Event {
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
}

/// Helper: snapshot with a real selection (highlight).
fn selection_snap(t: f64, path: &str) -> Event {
    selection_snap_with(t, path, "selected\n")
}

fn selection_snap_with(t: f64, path: &str, content: &str) -> Event {
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
}

/// Consecutive cursor-only snapshots compress to keep only the last before speech.
#[test]
fn consecutive_cursor_snapshots() {
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
fn preserves_selection_snapshots() {
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
fn keeps_diffs_between_snapshots() {
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
    // Two selection snapshots with no words — both files should appear.
    let mut events = vec![
        selection_snap_with(1.0, "a.rs", "fn a()\n"),
        selection_snap_with(2.0, "b.rs", "fn b()\n"),
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
