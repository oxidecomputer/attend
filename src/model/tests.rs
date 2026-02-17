use std::collections::HashMap;
use std::io;

use proptest::prelude::*;

use super::*;

// ── Reference oracle ────────────────────────────────────────────────────────

fn reference_position(content: &[u8], offset: usize) -> Position {
    let clamped = offset.min(content.len());
    let mut line = 1;
    let mut col = 1;
    let mut after_cr = false;
    for &b in &content[..clamped] {
        match b {
            b'\n' if after_cr => {
                after_cr = false;
            }
            b'\n' => {
                line += 1;
                col = 1;
            }
            b'\r' => {
                line += 1;
                col = 1;
                after_cr = true;
            }
            _ => {
                col += 1;
                after_cr = false;
            }
        }
    }
    Position { line, col }
}

// ── Strategies ──────────────────────────────────────────────────────────────

fn arb_position() -> impl Strategy<Value = Position> {
    (1..20usize, 1..50usize).prop_map(|(line, col)| Position { line, col })
}

fn arb_selection() -> impl Strategy<Value = Selection> {
    (arb_position(), arb_position()).prop_map(|(start, end)| Selection { start, end })
}

fn arb_file_entry() -> impl Strategy<Value = FileEntry> {
    ("[a-e]\\.rs", prop::collection::vec(arb_selection(), 0..4))
        .prop_map(|(path, selections)| FileEntry { path, selections })
}

fn arb_editor_state() -> impl Strategy<Value = EditorState> {
    (
        prop::collection::vec(arb_file_entry(), 0..6),
        prop::collection::vec("[a-c]/term", 0..3),
    )
        .prop_map(|(files, terminals)| {
            // Deduplicate files by path (keep first occurrence)
            let mut seen = std::collections::HashSet::new();
            let files = files
                .into_iter()
                .filter(|f| seen.insert(f.path.clone()))
                .collect();
            EditorState { files, terminals }
        })
}

/// Generate file content with consistent newline style and sorted unique offsets.
fn arb_content_and_offsets() -> impl Strategy<Value = (Vec<u8>, Vec<usize>)> {
    // Generate line segments (no \r or \n inside)
    let segments = prop::collection::vec(
        prop::collection::vec(
            prop::num::u8::ANY.prop_filter("no CR/LF", |&b| b != b'\r' && b != b'\n'),
            0..30,
        ),
        1..10,
    );
    // Choose newline style: 0 = \n, 1 = \r\n, 2 = \r
    let nl_style = 0..3u8;

    (segments, nl_style).prop_flat_map(|(segs, nl)| {
        let newline: &[u8] = match nl {
            0 => b"\n",
            1 => b"\r\n",
            _ => b"\r",
        };
        let mut content = Vec::new();
        for (i, seg) in segs.iter().enumerate() {
            if i > 0 {
                content.extend_from_slice(newline);
            }
            content.extend_from_slice(seg);
        }
        let len = content.len();
        let offsets = prop::collection::vec(0..=len, 0..8).prop_map(|mut v| {
            v.sort_unstable();
            v.dedup();
            v
        });
        (Just(content), offsets)
    })
}

/// Generate raw byte-offset pairs within the given content length.
fn arb_raw_pairs(content_len: usize) -> impl Strategy<Value = Vec<(i64, i64)>> {
    if content_len == 0 {
        return Just(vec![(0, 0)]).boxed();
    }
    prop::collection::vec((0..content_len as i64, 0..content_len as i64), 1..6).boxed()
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// Collect (path, sorted multiset of (start, end)) for comparison.
fn file_multiset(files: &[FileEntry]) -> HashMap<String, Vec<(Position, Position)>> {
    let mut map: HashMap<String, Vec<(Position, Position)>> = HashMap::new();
    for f in files {
        let mut sels: Vec<_> = f
            .selections
            .iter()
            .map(|s| (s.start.clone(), s.end.clone()))
            .collect();
        sels.sort_by(|a, b| a.0.line.cmp(&b.0.line).then(a.0.col.cmp(&b.0.col)));
        map.entry(f.path.clone()).or_default().extend(sels);
    }
    map
}

// ── reorder_by_newness: file-level properties ───────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// (a) Preservation: multiset of (path, selections) is identical before/after.
    #[test]
    fn reorder_preserves_files(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let before = file_multiset(&current.files);
        let mut state = current;
        reorder_by_newness(&mut state, &previous);
        let after = file_multiset(&state.files);
        prop_assert_eq!(before, after);
    }

    /// (b) Touched-before-unchanged partition.
    #[test]
    fn reorder_touched_before_unchanged(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let prev_map: HashMap<&str, &Vec<Selection>> = previous
            .files.iter().map(|f| (f.path.as_str(), &f.selections)).collect();
        let mut state = current;
        reorder_by_newness(&mut state, &previous);

        let mut seen_unchanged = false;
        for f in &state.files {
            let is_unchanged = prev_map.get(f.path.as_str())
                .is_some_and(|prev_sels| *prev_sels == &f.selections);
            if is_unchanged {
                seen_unchanged = true;
            } else {
                prop_assert!(!seen_unchanged,
                    "touched file {:?} appeared after unchanged file", f.path);
            }
        }
    }

    /// (c) Relative order within groups is preserved.
    #[test]
    fn reorder_relative_order(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let prev_map: HashMap<&str, (usize, &Vec<Selection>)> = previous
            .files.iter().enumerate()
            .map(|(i, f)| (f.path.as_str(), (i, &f.selections))).collect();

        let input_order: Vec<String> = current.files.iter().map(|f| f.path.clone()).collect();
        let mut state = current;
        reorder_by_newness(&mut state, &previous);

        // Touched files: should maintain relative input order
        let touched: Vec<&str> = state.files.iter()
            .filter(|f| {
                prev_map.get(f.path.as_str())
                    .is_none_or(|(_, prev_sels)| *prev_sels != &f.selections)
            })
            .map(|f| f.path.as_str())
            .collect();
        let touched_input_order: Vec<&str> = input_order.iter()
            .filter(|p| touched.contains(&p.as_str()))
            .map(|p| p.as_str())
            .collect();
        prop_assert_eq!(touched, touched_input_order);

        // Unchanged files: should be sorted by previous index
        let unchanged_prev_indices: Vec<usize> = state.files.iter()
            .filter_map(|f| {
                prev_map.get(f.path.as_str()).and_then(|(idx, prev_sels)| {
                    if *prev_sels == &f.selections { Some(*idx) } else { None }
                })
            })
            .collect();
        let mut sorted = unchanged_prev_indices.clone();
        sorted.sort();
        prop_assert_eq!(unchanged_prev_indices, sorted);
    }

    /// (d) Idempotency: reorder by self is identity.
    #[test]
    fn reorder_idempotent(state in arb_editor_state()) {
        let original = state.files.clone();
        let mut ed = EditorState {
            files: state.files.clone(),
            terminals: state.terminals.clone(),
        };
        reorder_by_newness(&mut ed, &state);
        prop_assert_eq!(
            ed.files.iter().map(|f| &f.path).collect::<Vec<_>>(),
            original.iter().map(|f| &f.path).collect::<Vec<_>>(),
        );
    }

    /// (e) Stability / cache round-trip: reorder(alphabetical, cached) == cached ordering.
    #[test]
    fn reorder_stability(cached in arb_editor_state()) {
        let mut alpha_files = cached.files.clone();
        alpha_files.sort_by(|a, b| a.path.cmp(&b.path));
        let mut alpha = EditorState {
            files: alpha_files,
            terminals: cached.terminals.clone(),
        };
        reorder_by_newness(&mut alpha, &cached);
        let expected_paths: Vec<&str> = cached.files.iter().map(|f| f.path.as_str()).collect();
        let actual_paths: Vec<&str> = alpha.files.iter().map(|f| f.path.as_str()).collect();
        prop_assert_eq!(actual_paths, expected_paths);
    }

    /// (f) Terminals are untouched.
    #[test]
    fn reorder_terminals_untouched(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let terminals_before = current.terminals.clone();
        let mut state = current;
        reorder_by_newness(&mut state, &previous);
        prop_assert_eq!(state.terminals, terminals_before);
    }
}

// ── reorder_by_newness: selection-level properties ──────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// (g) Unchanged file: selection identity.
    #[test]
    fn reorder_unchanged_file_selections(
        current in arb_editor_state(),
    ) {
        // Use current as both state and previous → all unchanged
        let mut state = EditorState {
            files: current.files.clone(),
            terminals: current.terminals.clone(),
        };
        reorder_by_newness(&mut state, &current);
        for (orig, reordered) in current.files.iter().zip(state.files.iter()) {
            prop_assert_eq!(&orig.selections, &reordered.selections);
        }
    }

    /// (h) New file: selection identity.
    #[test]
    fn reorder_new_file_selections(
        current in arb_editor_state(),
    ) {
        let empty = EditorState { files: vec![], terminals: vec![] };
        let original_sels: Vec<Vec<Selection>> = current.files.iter()
            .map(|f| f.selections.clone())
            .collect();
        let mut state = current;
        reorder_by_newness(&mut state, &empty);
        for (orig_sels, f) in original_sels.iter().zip(state.files.iter()) {
            prop_assert_eq!(orig_sels, &f.selections);
        }
    }

    /// (j) Touched file: selection preservation (multiset).
    #[test]
    fn reorder_touched_selection_preservation(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let before_sels: HashMap<String, Vec<(Position, Position)>> = current.files.iter()
            .map(|f| (f.path.clone(), f.selections.iter()
                .map(|s| (s.start.clone(), s.end.clone())).collect()))
            .collect();
        let mut state = current;
        reorder_by_newness(&mut state, &previous);
        for f in &state.files {
            if let Some(orig) = before_sels.get(&f.path) {
                let mut after: Vec<_> = f.selections.iter()
                    .map(|s| (s.start.clone(), s.end.clone())).collect();
                let mut expected = orig.clone();
                after.sort_by(|a, b| a.0.line.cmp(&b.0.line).then(a.0.col.cmp(&b.0.col)));
                expected.sort_by(|a, b| a.0.line.cmp(&b.0.line).then(a.0.col.cmp(&b.0.col)));
                prop_assert_eq!(after, expected,
                    "selection multiset mismatch for {:?}", f.path);
            }
        }
    }

    /// (i) Touched file: selection partition (new before unchanged).
    #[test]
    fn reorder_touched_selection_partition(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let prev_map: HashMap<&str, &Vec<Selection>> = previous
            .files.iter().map(|f| (f.path.as_str(), &f.selections)).collect();
        let mut state = current;
        reorder_by_newness(&mut state, &previous);

        for f in &state.files {
            if let Some(prev_sels) = prev_map.get(f.path.as_str()) {
                if *prev_sels == &f.selections {
                    continue; // unchanged
                }
                let prev_set: std::collections::HashSet<(&Position, &Position)> = prev_sels
                    .iter().map(|s| (&s.start, &s.end)).collect();
                let mut seen_unchanged = false;
                for sel in &f.selections {
                    let is_old = prev_set.contains(&(&sel.start, &sel.end));
                    if is_old {
                        seen_unchanged = true;
                    } else {
                        prop_assert!(!seen_unchanged,
                            "new selection appeared after unchanged in {:?}", f.path);
                    }
                }
            }
        }
    }
}

// ── offsets_to_positions: reference oracle ───────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Streaming implementation matches reference oracle for all newline styles.
    #[test]
    fn offsets_matches_reference((content, offsets) in arb_content_and_offsets()) {
        let reader = io::Cursor::new(&content);
        let positions = offsets_to_positions(reader, &offsets).unwrap();
        for (i, &off) in offsets.iter().enumerate() {
            let expected = reference_position(&content, off);
            prop_assert_eq!(&positions[i], &expected,
                "mismatch at offset {} (content len {})", off, content.len());
        }
    }
}

// ── resolve_selections_from_reader ──────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// (a) Dedup count: output length == number of unique (s, e) pairs.
    #[test]
    fn resolve_dedup_count((content, _) in arb_content_and_offsets()) {
        prop_assume!(!content.is_empty());
        let pairs = arb_raw_pairs(content.len());
        proptest::test_runner::TestRunner::new(Default::default())
            .run(&pairs, |raw| {
                let reader = io::Cursor::new(&content);
                let result = resolve_selections_from_reader(reader, &raw).unwrap();
                let mut unique = raw.clone();
                unique.sort();
                unique.dedup();
                prop_assert_eq!(result.len(), unique.len());
                Ok(())
            })
            .unwrap();
    }

    /// (b) Consistency with reference oracle.
    #[test]
    fn resolve_matches_reference((content, _) in arb_content_and_offsets()) {
        prop_assume!(!content.is_empty());
        let pairs = arb_raw_pairs(content.len());
        proptest::test_runner::TestRunner::new(Default::default())
            .run(&pairs, |raw| {
                let reader = io::Cursor::new(&content);
                let result = resolve_selections_from_reader(reader, &raw).unwrap();
                let mut unique = raw.clone();
                unique.sort();
                unique.dedup();
                for (sel, &(s, e)) in result.iter().zip(unique.iter()) {
                    let exp_start = reference_position(&content, s as usize);
                    let exp_end = reference_position(&content, e as usize);
                    prop_assert_eq!(&sel.start, &exp_start,
                        "start mismatch for pair ({}, {})", s, e);
                    prop_assert_eq!(&sel.end, &exp_end,
                        "end mismatch for pair ({}, {})", s, e);
                }
                Ok(())
            })
            .unwrap();
    }
}

// ── JSON round-trip ─────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    /// Identity: from_str(to_string(state)) == state.
    #[test]
    fn json_round_trip(state in arb_editor_state()) {
        let json = serde_json::to_string(&state).unwrap();
        let recovered: EditorState = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(state.files, recovered.files);
        prop_assert_eq!(state.terminals, recovered.terminals);
    }
}
