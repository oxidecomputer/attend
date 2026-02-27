use proptest::prelude::*;

use super::super::*;

use crate::narrate::render::{SnipConfig, format_markdown};
use crate::state::{Col, FileEntry, Line, Position, Selection};

/// Convert seconds to a UTC timestamp (for test brevity).
fn ts(secs: f64) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH + chrono::Duration::milliseconds((secs * 1000.0) as i64)
}

// ── Strategies ──────────────────────────────────────────────────────────────

fn arb_words() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z ]{1,30}").prop_map(|(t, text)| Event::Words {
        timestamp: ts(t),
        text,
    })
}

fn arb_language() -> impl Strategy<Value = Option<String>> {
    prop_oneof![
        4 => Just(None),
        1 => Just(Some("rust".to_string())),
        1 => Just(Some("python".to_string())),
        1 => Just(Some("javascript".to_string())),
        1 => Just(Some("c".to_string())),
    ]
}

fn arb_cursor_snapshot() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z]{1,8}\\.rs", arb_language()).prop_map(|(t, path, language)| {
        let pos = Position {
            line: Line::new(1).unwrap(),
            col: Col::new(1).unwrap(),
        };
        let sel = Selection {
            start: pos,
            end: pos,
        };
        Event::EditorSnapshot {
            timestamp: ts(t),
            last_seen: ts(t),
            files: vec![FileEntry {
                path: path.clone().into(),
                selections: vec![sel],
            }],
            regions: vec![CapturedRegion {
                path,
                content: "x\n".to_string(),
                first_line: 1,
                selections: vec![sel],
                language,
            }],
        }
    })
}

fn arb_selection_snapshot() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z]{1,8}\\.rs", arb_language()).prop_map(|(t, path, language)| {
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
            timestamp: ts(t),
            last_seen: ts(t),
            files: vec![FileEntry {
                path: path.clone().into(),
                selections: vec![sel],
            }],
            regions: vec![CapturedRegion {
                path,
                content: "selected content\n".to_string(),
                first_line: 1,
                selections: vec![sel],
                language,
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
            timestamp: ts(t),
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
            timestamp: ts(t),
            last_seen: ts(t),
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
            timestamp: ts(t),
            last_seen: ts(t),
            url,
            title,
            plain_text: text.clone(),
            text,
        })
}

fn arb_shell_command() -> impl Strategy<Value = Event> {
    (
        0.0..100.0f64,
        prop_oneof!["fish", "zsh"],
        prop_oneof!["cargo test", "cargo fmt", "make build", "git status"],
        prop_oneof!["/home/user/project", "/tmp/test", "."],
        prop_oneof![
            Just((None, None)),
            (0..3i32, 0.0..10.0f64).prop_map(|(s, d)| (Some(s), Some(d)))
        ],
    )
        .prop_map(|(t, shell, command, cwd, (exit_status, duration_secs))| {
            Event::ShellCommand {
                timestamp: ts(t),
                shell,
                command,
                cwd,
                exit_status,
                duration_secs,
            }
        })
}

fn arb_clipboard_text() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z ]{1,30}").prop_map(|(t, text)| Event::ClipboardSelection {
        timestamp: ts(t),
        content: ClipboardContent::Text { text },
    })
}

fn arb_clipboard_image() -> impl Strategy<Value = Event> {
    (0.0..100.0f64, "[a-z]{1,8}").prop_map(|(t, name)| Event::ClipboardSelection {
        timestamp: ts(t),
        content: ClipboardContent::Image {
            path: format!("/tmp/staging/clipboard/{name}.png"),
        },
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
        1 => arb_shell_command(),
        1 => arb_clipboard_text(),
        1 => arb_clipboard_image(),
    ]
}

// ── Cross-run subsumption sequence strategies ───────────────────────────────
//
// Generate three-event sequences mimicking real progressive selection:
// partial selection → words (speech while dragging) → wider selection.
// The words event lands between the two selections, splitting them into
// different runs. The global subsumption pass must recognize the pattern
// and drop the partial. Random text from arb_event() almost never produces
// substring relationships, so these ensure the subsumption paths are
// reliably exercised.

/// Partial ExternalSelection → Words → wider ExternalSelection (same source).
fn arb_cross_run_ext() -> impl Strategy<Value = Vec<Event>> {
    (
        0.0..96.0f64,
        0.2..1.0f64,
        0.2..1.0f64,
        prop_oneof!["iTerm2", "Safari", "Firefox"],
        "[a-z ]{1,15}",
        "[a-z ]{1,15}",
        "[a-z ]{1,20}",
    )
        .prop_map(|(t, dt_word, dt_wide, app, prefix, suffix, speech)| {
            let t_word = t + dt_word;
            let t_wide = t_word + dt_wide;
            let wide = format!("{prefix}{suffix}");
            vec![
                Event::ExternalSelection {
                    timestamp: ts(t),
                    last_seen: ts(t),
                    app: app.clone(),
                    window_title: "window".to_string(),
                    text: prefix,
                },
                Event::Words {
                    timestamp: ts(t_word),
                    text: speech,
                },
                Event::ExternalSelection {
                    timestamp: ts(t_wide),
                    last_seen: ts(t_wide),
                    app,
                    window_title: "window".to_string(),
                    text: wide,
                },
            ]
        })
}

/// Partial BrowserSelection → Words → wider BrowserSelection (same URL).
fn arb_cross_run_browser() -> impl Strategy<Value = Vec<Event>> {
    (
        0.0..96.0f64,
        0.2..1.0f64,
        0.2..1.0f64,
        prop_oneof![
            "https://docs.rs/tokio",
            "https://example.com",
            "https://github.com/foo"
        ],
        "[a-z ]{1,15}",
        "[a-z ]{1,15}",
        "[a-z ]{1,20}",
    )
        .prop_map(|(t, dt_word, dt_wide, url, prefix, suffix, speech)| {
            let t_word = t + dt_word;
            let t_wide = t_word + dt_wide;
            let wide = format!("{prefix}{suffix}");
            vec![
                Event::BrowserSelection {
                    timestamp: ts(t),
                    last_seen: ts(t),
                    url: url.clone(),
                    title: "Page".to_string(),
                    plain_text: prefix.clone(),
                    text: prefix,
                },
                Event::Words {
                    timestamp: ts(t_word),
                    text: speech,
                },
                Event::BrowserSelection {
                    timestamp: ts(t_wide),
                    last_seen: ts(t_wide),
                    url,
                    title: "Page".to_string(),
                    plain_text: wide.clone(),
                    text: wide,
                },
            ]
        })
}

/// Narrow EditorSnapshot → Words → wider EditorSnapshot (same file path).
fn arb_cross_run_snapshot() -> impl Strategy<Value = Vec<Event>> {
    (
        0.0..96.0f64,
        0.2..1.0f64,
        0.2..1.0f64,
        "[a-z]{1,8}\\.rs",
        "[a-z ]{1,15}",
        "[a-z ]{1,15}",
        "[a-z ]{1,20}",
        arb_language(),
    )
        .prop_map(
            |(t, dt_word, dt_wide, path, prefix, suffix, speech, language)| {
                let t_word = t + dt_word;
                let t_wide = t_word + dt_wide;
                let wide = format!("{prefix}{suffix}");
                let start = Position {
                    line: Line::new(1).unwrap(),
                    col: Col::new(1).unwrap(),
                };
                let end = Position {
                    line: Line::new(5).unwrap(),
                    col: Col::new(10).unwrap(),
                };
                let sel = Selection { start, end };
                let make_snap = |ts_val: f64, content: String| Event::EditorSnapshot {
                    timestamp: ts(ts_val),
                    last_seen: ts(ts_val),
                    files: vec![FileEntry {
                        path: path.clone().into(),
                        selections: vec![sel],
                    }],
                    regions: vec![CapturedRegion {
                        path: path.clone(),
                        content,
                        first_line: 1,
                        selections: vec![sel],
                        language: language.clone(),
                    }],
                };
                vec![
                    make_snap(t, prefix),
                    Event::Words {
                        timestamp: ts(t_word),
                        text: speech,
                    },
                    make_snap(t_wide, wide),
                ]
            },
        )
}

/// Held ExternalSelection: selection persists (last_seen near wider's timestamp).
/// Exercises the last_seen-based gap calculation in subsumption.
fn arb_cross_run_ext_held() -> impl Strategy<Value = Vec<Event>> {
    (
        0.0..90.0f64,
        1.0..5.0f64,
        0.2..1.0f64,
        prop_oneof!["iTerm2", "Safari", "Firefox"],
        "[a-z ]{1,15}",
        "[a-z ]{1,15}",
        "[a-z ]{1,20}",
    )
        .prop_map(|(t, hold_dur, dt_wide, app, prefix, suffix, speech)| {
            let t_word = t + hold_dur * 0.5;
            let t_last_seen = t + hold_dur;
            let t_wide = t_last_seen + dt_wide;
            let wide = format!("{prefix}{suffix}");
            vec![
                Event::ExternalSelection {
                    timestamp: ts(t),
                    last_seen: ts(t_last_seen),
                    app: app.clone(),
                    window_title: "window".to_string(),
                    text: prefix,
                },
                Event::Words {
                    timestamp: ts(t_word),
                    text: speech,
                },
                Event::ExternalSelection {
                    timestamp: ts(t_wide),
                    last_seen: ts(t_wide),
                    app,
                    window_title: "window".to_string(),
                    text: wide,
                },
            ]
        })
}

fn arb_events() -> impl Strategy<Value = Vec<Event>> {
    (
        proptest::collection::vec(arb_event(), 0..15),
        proptest::collection::vec(
            prop_oneof![
                arb_cross_run_ext(),
                arb_cross_run_ext_held(),
                arb_cross_run_browser(),
                arb_cross_run_snapshot(),
            ],
            0..3,
        ),
    )
        .prop_map(|(mut events, runs)| {
            for run in runs {
                events.extend(run);
            }
            events
        })
}

// ── compress_and_merge ──────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// compress_and_merge produces events sorted by timestamp.
    #[test]
    fn merge_output_sorted(mut events in arb_events()) {
        compress_and_merge(&mut events);
        for w in events.windows(2) {
            prop_assert!(
                w[0].timestamp() <= w[1].timestamp(),
                "output not sorted: {:?} > {:?}",
                w[0].timestamp(),
                w[1].timestamp()
            );
        }
    }

    /// compress_and_merge preserves all Words events in chronological order.
    #[test]
    fn merge_preserves_words_in_order(events in arb_events()) {
        // Collect words in the order they'd appear after sorting by timestamp
        // (the first thing compress_and_merge does).
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| a.timestamp().cmp(&b.timestamp()));
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
                a.timestamp() == b.timestamp(),
                "idempotency violated: timestamp changed"
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

        for path in &input_selection_paths {
            prop_assert!(
                output_paths.contains(path),
                "post-word selection path {:?} lost during compression",
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

    /// Every input ExternalSelection's text is either present in the output
    /// or is a substring of a surviving ExternalSelection from the same
    /// (app, window_title) source. No text is silently lost.
    #[test]
    fn merge_preserves_ext_selection_content(events in arb_events()) {
        let input_texts: Vec<(String, String, String)> = events
            .iter()
            .filter_map(|e| match e {
                Event::ExternalSelection { app, window_title, text, .. } =>
                    Some((app.clone(), window_title.clone(), text.clone())),
                _ => None,
            })
            .collect();

        let mut merged = events;
        compress_and_merge(&mut merged);

        let output_texts: Vec<(String, String, String)> = merged
            .iter()
            .filter_map(|e| match e {
                Event::ExternalSelection { app, window_title, text, .. } =>
                    Some((app.clone(), window_title.clone(), text.clone())),
                _ => None,
            })
            .collect();

        for (app, wt, text) in &input_texts {
            let covered = output_texts.iter().any(|(oa, owt, ot)| {
                oa == app && owt == wt && (ot == text || ot.contains(text.as_str()))
            });
            prop_assert!(
                covered,
                "input text {:?} from ({}, {}) not covered in output",
                text, app, wt
            );
        }
    }

    /// Every input BrowserSelection's text is either present in the output
    /// or is a substring of a surviving BrowserSelection from the same url.
    /// Mirrors the `merge_preserves_ext_selection_content` invariant.
    #[test]
    fn merge_preserves_browser_selection_content(events in arb_events()) {
        let input_texts: Vec<(String, String)> = events
            .iter()
            .filter_map(|e| match e {
                Event::BrowserSelection { url, text, .. } =>
                    Some((url.clone(), text.clone())),
                _ => None,
            })
            .collect();

        let mut merged = events;
        compress_and_merge(&mut merged);

        let output_texts: Vec<(String, String)> = merged
            .iter()
            .filter_map(|e| match e {
                Event::BrowserSelection { url, text, .. } =>
                    Some((url.clone(), text.clone())),
                _ => None,
            })
            .collect();

        for (url, text) in &input_texts {
            let covered = output_texts.iter().any(|(ou, ot)| {
                ou == url && (ot == text || ot.contains(text.as_str()))
            });
            // Allow cross-type dedup: a BrowserSelection may be removed if
            // an ExternalSelection with matching trimmed text exists nearby.
            let cross_type_deduped = !covered && merged.iter().any(|e| {
                matches!(e, Event::ExternalSelection { text: t, .. } if t.trim() == text.trim())
            });
            prop_assert!(
                covered || cross_type_deduped,
                "input browser text {:?} from {} not covered in output",
                text, url
            );
        }
    }

    /// Each diff path from input appears in output unless the net change
    /// for that path in its wordless run is empty (old == new).
    #[test]
    fn merge_preserves_diff_paths(events in arb_events()) {
        // Sort input the same way compress_and_merge does.
        let mut sorted = events.clone();
        sorted.sort_by(|a, b| a.timestamp().cmp(&b.timestamp()));

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

    /// Every input EditorSnapshot region's content (from snapshots with real
    /// selections) either appears in some output EditorSnapshot region with
    /// the same path, or is a substring of one. Cross-run subsumption may
    /// drop a snapshot, but only when a later snapshot fully covers it.
    #[test]
    fn merge_preserves_snapshot_content(events in arb_events()) {
        let input_regions: Vec<(String, String)> = events
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { files, regions, .. }
                    if files.iter().any(|f| f.selections.iter().any(|s| !s.is_cursor_like())) =>
                {
                    Some(
                        regions
                            .iter()
                            .map(|r| (r.path.clone(), r.content.clone()))
                            .collect::<Vec<_>>(),
                    )
                }
                _ => None,
            })
            .flatten()
            .collect();

        let mut merged = events;
        compress_and_merge(&mut merged);

        let output_regions: Vec<(String, String)> = merged
            .iter()
            .filter_map(|e| match e {
                Event::EditorSnapshot { regions, .. } => Some(
                    regions
                        .iter()
                        .map(|r| (r.path.clone(), r.content.clone()))
                        .collect::<Vec<_>>(),
                ),
                _ => None,
            })
            .flatten()
            .collect();

        for (path, content) in &input_regions {
            let covered = output_regions.iter().any(|(op, oc)| {
                op == path && (oc == content || oc.contains(content.as_str()))
            });
            prop_assert!(
                covered,
                "input snapshot region {:?} content {:?} not covered in output",
                path,
                content
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

// ── Clipboard-specific prop tests ────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// After compress_and_merge, no text clipboard survives whose normalized
    /// text equals any ExternalSelection or BrowserSelection plain_text in the
    /// output.
    #[test]
    fn prop_clipboard_in_merge_pipeline(mut events in arb_events()) {
        compress_and_merge(&mut events);

        // Collect normalized texts from richer sources in the output,
        // excluding empty strings (whitespace-only normalizes to "" and
        // would vacuously match any other whitespace-only content).
        let mut richer_texts: Vec<String> = Vec::new();
        for e in &events {
            match e {
                Event::ExternalSelection { text, .. } => {
                    let norm = normalize_text(text);
                    if !norm.is_empty() {
                        richer_texts.push(norm);
                    }
                }
                Event::BrowserSelection { plain_text, .. } => {
                    let norm = normalize_text(plain_text);
                    if !norm.is_empty() {
                        richer_texts.push(norm);
                    }
                }
                _ => {}
            }
        }

        // Check: no text clipboard should match a richer source.
        // Skip empty normalized text: whitespace-only content normalizes to ""
        // and would vacuously match any other whitespace-only source.
        for e in &events {
            if let Event::ClipboardSelection {
                content: ClipboardContent::Text { text },
                ..
            } = e
            {
                let norm = normalize_text(text);
                if norm.is_empty() {
                    continue;
                }
                prop_assert!(
                    !richer_texts.contains(&norm),
                    "clipboard text {:?} (normalized: {:?}) should have been deduped \
                     against a richer source",
                    text,
                    norm
                );
            }
        }
    }

    /// Subsumption is asymmetric: a clipboard event may be dropped when a
    /// richer type contains it, but a richer type is never dropped by a
    /// clipboard event containing it.
    #[test]
    fn prop_clipboard_subsumption_asymmetric(
        (t, clipboard_text_val, ext_text, app) in (
            0.0..95.0f64,
            "[a-z ]{1,10}",
            "[a-z ]{1,10}",
            prop_oneof!["iTerm2", "Safari"],
        )
    ) {
        // Case 1: clipboard is a substring of external → clipboard may be dropped.
        let wide_ext = format!("{ext_text}{clipboard_text_val}");
        let mut events1 = vec![
            Event::ClipboardSelection {
                timestamp: ts(t),
                content: ClipboardContent::Text { text: clipboard_text_val.clone() },
            },
            Event::ExternalSelection {
                timestamp: ts(t + 0.5),
                last_seen: ts(t + 0.5),
                app: app.clone(),
                window_title: "w".to_string(),
                text: wide_ext,
            },
        ];
        compress_and_merge(&mut events1);
        // External should always survive.
        prop_assert!(
            events1.iter().any(|e| matches!(e, Event::ExternalSelection { .. })),
            "external should survive even when clipboard is its substring"
        );

        // Case 2: external is a substring of clipboard → external must survive.
        let wide_clip = format!("{clipboard_text_val}{ext_text}");
        let mut events2 = vec![
            Event::ExternalSelection {
                timestamp: ts(t),
                last_seen: ts(t),
                app,
                window_title: "w".to_string(),
                text: ext_text,
            },
            Event::ClipboardSelection {
                timestamp: ts(t + 0.5),
                content: ClipboardContent::Text { text: wide_clip },
            },
        ];
        compress_and_merge(&mut events2);
        // External must never be subsumed by clipboard.
        prop_assert!(
            events2.iter().any(|e| matches!(e, Event::ExternalSelection { .. })),
            "external must NOT be subsumed by clipboard, even if clipboard contains it"
        );
    }
}
