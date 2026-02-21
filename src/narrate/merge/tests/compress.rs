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

// ── Single-event pass-through ─────────────────────────────────────────────

/// A single Words event passes through unchanged.
#[test]
fn single_word_event() {
    let mut events = vec![Event::Words {
        offset_secs: 1.0,
        text: "hello".to_string(),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert_eq!(md, "hello\n");
}

/// A single cursor-only snapshot survives (code-only narration, no speech to trigger drop).
#[test]
fn single_cursor_snapshot() {
    let mut events = vec![cursor_snap(1.0, "solo.rs")];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("solo.rs"), "sole cursor snap should survive");
}

/// A single selection snapshot survives.
#[test]
fn single_selection_snapshot() {
    let mut events = vec![selection_snap(1.0, "sel.rs")];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("sel.rs"), "sole selection snap should survive");
}

/// A single diff event survives.
#[test]
fn single_diff_event() {
    let mut events = vec![Event::FileDiff {
        offset_secs: 1.0,
        path: "one.rs".to_string(),
        old: "old\n".to_string(),
        new: "new\n".to_string(),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("one.rs"), "sole diff should survive");
    assert!(md.contains("+new"), "diff content should render");
}

// ── All-cursor-only with no words ─────────────────────────────────────────

/// All cursor-only snapshots with no speech: only the last survives
/// (code-only narration keeps everything, but compress_snapshots collapses
/// the run to just the last cursor snapshot).
#[test]
fn all_cursor_only_no_words() {
    let mut events = vec![
        cursor_snap(1.0, "a.rs"),
        cursor_snap(2.0, "b.rs"),
        cursor_snap(3.0, "c.rs"),
        cursor_snap(4.0, "d.rs"),
    ];
    compress_and_merge(&mut events);
    // Only the last cursor-only snapshot should remain.
    assert_eq!(events.len(), 1, "only one snapshot should survive");
    assert_eq!(events[0].offset_secs(), 4.0, "should be the last one");
}

// ── All-diffs scenarios ───────────────────────────────────────────────────

/// Multiple diffs to the same path with no words between: merged to net change.
#[test]
fn all_diffs_same_path_no_words() {
    let mut events = vec![
        Event::FileDiff {
            offset_secs: 1.0,
            path: "f.rs".to_string(),
            old: "v1\n".to_string(),
            new: "v2\n".to_string(),
        },
        Event::FileDiff {
            offset_secs: 2.0,
            path: "f.rs".to_string(),
            old: "v2\n".to_string(),
            new: "v3\n".to_string(),
        },
        Event::FileDiff {
            offset_secs: 3.0,
            path: "f.rs".to_string(),
            old: "v3\n".to_string(),
            new: "v4\n".to_string(),
        },
    ];
    compress_and_merge(&mut events);
    assert_eq!(events.len(), 1, "should merge to one diff");
    if let Event::FileDiff { old, new, .. } = &events[0] {
        assert_eq!(old, "v1\n", "old should be from first diff");
        assert_eq!(new, "v4\n", "new should be from last diff");
    } else {
        panic!("expected FileDiff");
    }
}

/// Multiple diffs to different paths in a wordless run: all paths survive.
#[test]
fn all_diffs_different_paths_no_words() {
    let mut events = vec![
        Event::FileDiff {
            offset_secs: 1.0,
            path: "a.rs".to_string(),
            old: "".to_string(),
            new: "a\n".to_string(),
        },
        Event::FileDiff {
            offset_secs: 2.0,
            path: "b.rs".to_string(),
            old: "".to_string(),
            new: "b\n".to_string(),
        },
        Event::FileDiff {
            offset_secs: 3.0,
            path: "c.rs".to_string(),
            old: "".to_string(),
            new: "c\n".to_string(),
        },
    ];
    compress_and_merge(&mut events);
    let paths: Vec<&str> = events
        .iter()
        .filter_map(|e| match e {
            Event::FileDiff { path, .. } => Some(path.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(paths.len(), 3, "all three distinct diffs survive");
    assert!(paths.contains(&"a.rs"));
    assert!(paths.contains(&"b.rs"));
    assert!(paths.contains(&"c.rs"));
}

// ── Interleaved words prevent merging ─────────────────────────────────────

/// Snapshots separated by Words are not merged with each other.
#[test]
fn interleaved_word_snap_not_merged() {
    let mut events = vec![
        selection_snap_with(1.0, "a.rs", "fn a()\n"),
        Event::Words {
            offset_secs: 2.0,
            text: "then".to_string(),
        },
        selection_snap_with(3.0, "b.rs", "fn b()\n"),
    ];
    compress_and_merge(&mut events);
    // Both snapshots should survive as separate events (not merged).
    let snap_count = events
        .iter()
        .filter(|e| matches!(e, Event::EditorSnapshot { .. }))
        .count();
    assert_eq!(snap_count, 2, "snapshots separated by words stay separate");
}

/// Diffs to the same path separated by words are not merged.
#[test]
fn diffs_separated_by_words_not_merged() {
    let mut events = vec![
        Event::FileDiff {
            offset_secs: 1.0,
            path: "f.rs".to_string(),
            old: "v1\n".to_string(),
            new: "v2\n".to_string(),
        },
        Event::Words {
            offset_secs: 2.0,
            text: "now".to_string(),
        },
        Event::FileDiff {
            offset_secs: 3.0,
            path: "f.rs".to_string(),
            old: "v2\n".to_string(),
            new: "v3\n".to_string(),
        },
    ];
    compress_and_merge(&mut events);
    let diff_count = events
        .iter()
        .filter(|e| matches!(e, Event::FileDiff { .. }))
        .count();
    assert_eq!(diff_count, 2, "diffs separated by words stay separate");
}

// ── No-op diff ────────────────────────────────────────────────────────────

/// A diff where old == new produces no output (render skips it).
#[test]
fn noop_diff_filtered_by_render() {
    let mut events = vec![Event::FileDiff {
        offset_secs: 1.0,
        path: "noop.rs".to_string(),
        old: "same\n".to_string(),
        new: "same\n".to_string(),
    }];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(
        !md.contains("noop.rs"),
        "no-op diff should produce no output"
    );
    assert!(!md.contains("```diff"), "no diff fence for no-op");
}

// ── Mixed run: diffs + snapshots in one wordless span ─────────────────────

/// A wordless run containing both diffs and snapshots: both survive after merge.
#[test]
fn mixed_diffs_and_snapshots_in_wordless_run() {
    let mut events = vec![
        selection_snap_with(1.0, "view.rs", "fn view()\n"),
        Event::FileDiff {
            offset_secs: 2.0,
            path: "edit.rs".to_string(),
            old: "old\n".to_string(),
            new: "new\n".to_string(),
        },
        selection_snap_with(3.0, "other.rs", "fn other()\n"),
    ];
    let md = format_markdown(&mut events, SnipConfig::default());
    assert!(md.contains("view.rs"), "first snapshot survives");
    assert!(md.contains("other.rs"), "second snapshot survives");
    assert!(md.contains("edit.rs"), "diff survives");
}

// ── Trailing cursor-only drop only when speech present ────────────────────

/// Trailing cursor snapshot is dropped when words are present.
#[test]
fn trailing_cursor_dropped_with_speech() {
    let mut events = vec![
        Event::Words {
            offset_secs: 1.0,
            text: "hello".to_string(),
        },
        cursor_snap(2.0, "trail.rs"),
    ];
    compress_and_merge(&mut events);
    assert_eq!(events.len(), 1, "trailing cursor dropped");
    assert!(matches!(events[0], Event::Words { .. }));
}

/// Trailing cursor snapshot is kept when no words are present (code-only).
#[test]
fn trailing_cursor_kept_without_speech() {
    let mut events = vec![selection_snap(1.0, "sel.rs"), cursor_snap(2.0, "trail.rs")];
    compress_and_merge(&mut events);
    // Both should survive (code-only narration).
    let has_trail = events
        .iter()
        .any(|e| matches!(e, Event::EditorSnapshot { rendered, .. } if rendered.iter().any(|r| r.path == "trail.rs")));
    assert!(has_trail, "trailing cursor kept in code-only narration");
}

// ── Out-of-order input sorted correctly ───────────────────────────────────

/// Events with out-of-order timestamps are sorted before merge logic runs.
#[test]
fn out_of_order_sorted_before_merge() {
    let mut events = vec![
        Event::Words {
            offset_secs: 5.0,
            text: "second".to_string(),
        },
        cursor_snap(3.0, "mid.rs"),
        Event::Words {
            offset_secs: 1.0,
            text: "first".to_string(),
        },
    ];
    compress_and_merge(&mut events);
    // After sorting: Words(1.0), cursor(3.0), Words(5.0)
    // cursor at 3.0 is between two words, so it survives.
    assert_eq!(events.len(), 3);
    assert!(
        events[0].offset_secs() <= events[1].offset_secs()
            && events[1].offset_secs() <= events[2].offset_secs(),
        "output must be sorted"
    );
}
