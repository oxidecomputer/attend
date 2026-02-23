use proptest::prelude::*;

use super::super::*;

use crate::narrate::render::{SnipConfig, format_markdown};
use crate::state::{Col, FileEntry, Line, Position, Selection};

// ── Strategies ──────────────────────────────────────────────────────────────

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
        let sel = Selection {
            start: pos,
            end: pos,
        };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.clone().into(),
                selections: vec![sel],
            }],
            regions: vec![CapturedRegion {
                path,
                content: "x\n".to_string(),
                first_line: 1,
                selections: vec![sel],
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
        let sel = Selection { start, end };
        Event::EditorSnapshot {
            offset_secs: t,
            files: vec![FileEntry {
                path: path.clone().into(),
                selections: vec![sel],
            }],
            regions: vec![CapturedRegion {
                path,
                content: "selected content\n".to_string(),
                first_line: 1,
                selections: vec![sel],
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

fn arb_ext_selection() -> impl Strategy<Value = Event> {
    (
        0.0..100.0f64,
        prop_oneof!["iTerm2", "Safari", "Firefox"],
        "[a-z ]{1,30}",
        "[a-z ]{1,30}",
    )
        .prop_map(|(t, app, title, text)| Event::ExternalSelection {
            offset_secs: t,
            app,
            window_title: title,
            text,
        })
}

fn arb_browser_selection() -> impl Strategy<Value = Event> {
    (
        0.0..100.0f64,
        prop_oneof![
            "https://docs.rs/tokio",
            "https://example.com",
            "https://github.com/foo"
        ],
        "[a-z ]{1,30}",
        "[a-z ]{1,30}",
    )
        .prop_map(|(t, url, title, text)| Event::BrowserSelection {
            offset_secs: t,
            url,
            title,
            text,
        })
}

fn arb_event() -> impl Strategy<Value = Event> {
    prop_oneof![
        3 => arb_words(),
        2 => arb_cursor_snapshot(),
        2 => arb_selection_snapshot(),
        1 => arb_diff(),
        1 => arb_ext_selection(),
        1 => arb_browser_selection(),
    ]
}

fn arb_events() -> impl Strategy<Value = Vec<Event>> {
    proptest::collection::vec(arb_event(), 0..20)
}

// ── compress_and_merge ──────────────────────────────────────────────────────

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

    /// compress_and_merge preserves all Words events in chronological order.
    #[test]
    fn merge_preserves_words_in_order(events in arb_events()) {
        // Collect words in the order they'd appear after sorting by offset
        // (the first thing compress_and_merge does).
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| a.offset_secs().partial_cmp(&b.offset_secs()).unwrap_or(std::cmp::Ordering::Equal));
        let words_before: Vec<String> = sorted
            .iter()
            .filter_map(|e| match e {
                Event::Words { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
        let mut merged = events;
        compress_and_merge(&mut merged);
        let words_after: Vec<String> = merged
            .iter()
            .filter_map(|e| match e {
                Event::Words { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect();
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
                Event::EditorSnapshot { files, regions, .. } => {
                    let has_selection = files.iter()
                        .any(|f| f.selections.iter().any(|s| !s.is_cursor_like()));
                    if has_selection {
                        Some(regions.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
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

        // Collect all region paths from output snapshots.
        let mut output_paths: Vec<String> = merged
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { regions, .. } => {
                    Some(regions.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
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

    /// All region file paths from selection snapshots appear in the output.
    #[test]
    fn merge_preserves_rendered_paths(events in arb_events()) {
        let mut input_paths: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { regions, files, .. }
                    if files.iter().any(|f| f.selections.iter().any(|s| !s.is_cursor_like())) =>
                {
                    Some(regions.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
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
                Event::EditorSnapshot { regions, .. } => {
                    Some(regions.iter().map(|r| r.path.clone()).collect::<Vec<_>>())
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

    /// compress_and_merge never increases the event count.
    #[test]
    fn merge_never_increases_count(events in arb_events()) {
        let original_len = events.len();
        let mut merged = events;
        compress_and_merge(&mut merged);
        prop_assert!(
            merged.len() <= original_len,
            "merge increased event count: {} -> {}",
            original_len,
            merged.len()
        );
    }

    /// No empty sentinels survive: EditorSnapshot with empty regions list
    /// and FileDiff with empty path are both removed.
    #[test]
    fn merge_no_empty_sentinels(events in arb_events()) {
        let mut merged = events;
        compress_and_merge(&mut merged);
        for (i, e) in merged.iter().enumerate() {
            match e {
                Event::EditorSnapshot { regions, .. } => {
                    prop_assert!(
                        !regions.is_empty(),
                        "empty regions list at index {i}"
                    );
                }
                Event::FileDiff { path, .. } => {
                    prop_assert!(
                        !path.is_empty(),
                        "empty diff path at index {i}"
                    );
                }
                Event::ExternalSelection { text, .. } => {
                    prop_assert!(
                        !text.is_empty(),
                        "empty external selection text at index {i}"
                    );
                }
                _ => {}
            }
        }
    }

    /// After compress_and_merge, no two consecutive cursor-only snapshots
    /// exist without an intervening Words event.
    #[test]
    fn merge_no_adjacent_cursor_only(events in arb_events()) {
        let mut merged = events;
        compress_and_merge(&mut merged);

        let is_cursor_only = |e: &Event| -> bool {
            let Event::EditorSnapshot { files, .. } = e else { return false };
            files.iter().all(|f| f.selections.iter().all(|s| s.is_cursor_like()))
        };

        let mut prev_was_cursor_only = false;
        for e in &merged {
            if matches!(e, Event::Words { .. }) {
                prev_was_cursor_only = false;
                continue;
            }
            let cursor = is_cursor_only(e);
            prop_assert!(
                !(prev_was_cursor_only && cursor),
                "two consecutive cursor-only snapshots found after merge"
            );
            if cursor {
                prev_was_cursor_only = true;
            }
        }
    }

    /// ExternalSelection events are never dropped by compress_and_merge
    /// (they have no compression logic that could eliminate them entirely,
    /// only dedup within a run).
    #[test]
    fn merge_preserves_ext_selection_text(events in arb_events()) {
        // Collect unique (app, text) pairs across all wordless runs.
        // Within a run, dedup keeps only one per (app, text), so count
        // unique pairs per run.
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| a.offset_secs().partial_cmp(&b.offset_secs()).unwrap_or(std::cmp::Ordering::Equal));

        let mut expected_count = 0usize;
        let mut run_pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

        let flush = |pairs: &mut std::collections::HashSet<(String, String)>, count: &mut usize| {
            *count += pairs.len();
            pairs.clear();
        };

        for e in &sorted {
            match e {
                Event::Words { .. } => flush(&mut run_pairs, &mut expected_count),
                Event::ExternalSelection { app, text, .. } => {
                    run_pairs.insert((app.clone(), text.clone()));
                }
                _ => {}
            }
        }
        flush(&mut run_pairs, &mut expected_count);

        let mut merged = events;
        compress_and_merge(&mut merged);
        let actual_count = merged.iter().filter(|e| matches!(e, Event::ExternalSelection { .. })).count();
        prop_assert_eq!(actual_count, expected_count);
    }

    /// BrowserSelection events are deduplicated per (url, text) within a run.
    /// The count after merge equals the number of unique (url, text) pairs
    /// per wordless run (minus any that were cross-type deduped with
    /// ExternalSelection, which is extremely rare with random text).
    #[test]
    fn merge_preserves_browser_selection_text(events in arb_events()) {
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| a.offset_secs().partial_cmp(&b.offset_secs()).unwrap_or(std::cmp::Ordering::Equal));

        let mut expected_count = 0usize;
        let mut run_pairs: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();

        let flush = |pairs: &mut std::collections::HashSet<(String, String)>, count: &mut usize| {
            *count += pairs.len();
            pairs.clear();
        };

        for e in &sorted {
            match e {
                Event::Words { .. } => flush(&mut run_pairs, &mut expected_count),
                Event::BrowserSelection { url, text, .. } => {
                    run_pairs.insert((url.clone(), text.clone()));
                }
                _ => {}
            }
        }
        flush(&mut run_pairs, &mut expected_count);

        let mut merged = events;
        compress_and_merge(&mut merged);
        let actual_count = merged.iter().filter(|e| matches!(e, Event::BrowserSelection { .. })).count();
        prop_assert_eq!(actual_count, expected_count);
    }

    /// Each diff path from input appears in output unless the net change
    /// for that path in its wordless run is empty (old == new).
    #[test]
    fn merge_preserves_diff_paths(events in arb_events()) {
        // Sort input the same way compress_and_merge does.
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| a.offset_secs().partial_cmp(&b.offset_secs()).unwrap_or(std::cmp::Ordering::Equal));

        // For each wordless run, compute expected surviving diff paths.
        let mut expected_paths: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut run_diffs: std::collections::HashMap<String, (String, String)> = std::collections::HashMap::new();

        let flush_run = |run_diffs: &mut std::collections::HashMap<String, (String, String)>,
                         expected: &mut std::collections::HashSet<String>| {
            for (path, (old, new)) in run_diffs.drain() {
                if old != new {
                    expected.insert(path);
                }
            }
        };

        for e in &sorted {
            match e {
                Event::Words { .. } => {
                    flush_run(&mut run_diffs, &mut expected_paths);
                }
                Event::FileDiff { path, old, new, .. } => {
                    run_diffs.entry(path.clone())
                        .and_modify(|entry| entry.1 = new.clone())
                        .or_insert((old.clone(), new.clone()));
                }
                _ => {}
            }
        }
        flush_run(&mut run_diffs, &mut expected_paths);

        let mut merged = events;
        compress_and_merge(&mut merged);
        let output_paths: std::collections::HashSet<String> = merged
            .iter()
            .filter_map(|e| match e {
                Event::FileDiff { path, .. } => Some(path.clone()),
                _ => None,
            })
            .collect();

        for path in &expected_paths {
            prop_assert!(
                output_paths.contains(path),
                "expected diff path {:?} missing from output",
                path
            );
        }
    }
}

// ── unified_diff ────────────────────────────────────────────────────────────

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

// ── render pipeline ─────────────────────────────────────────────────────────

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
