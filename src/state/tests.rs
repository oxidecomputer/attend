use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use proptest::prelude::*;

use super::{EditorState, FileEntry, Position, Selection};
use crate::editor::RawEditor;

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
    Position::of(line, col).unwrap()
}

// ── Strategies ──────────────────────────────────────────────────────────────

fn arb_position() -> impl Strategy<Value = Position> {
    (1..20usize, 1..50usize).prop_map(|(line, col)| Position::of(line, col).unwrap())
}

fn arb_selection() -> impl Strategy<Value = Selection> {
    (arb_position(), arb_position()).prop_map(|(start, end)| Selection { start, end })
}

fn arb_file_entry() -> impl Strategy<Value = FileEntry> {
    ("[a-e]\\.rs", prop::collection::vec(arb_selection(), 0..4)).prop_map(|(path, selections)| {
        FileEntry {
            path: PathBuf::from(path),
            selections,
        }
    })
}

fn arb_editor_state() -> impl Strategy<Value = EditorState> {
    prop::collection::vec(arb_file_entry(), 0..6).prop_map(|files| {
        // Deduplicate files by path (keep first occurrence)
        let mut seen = std::collections::HashSet::new();
        let files = files
            .into_iter()
            .filter(|f| seen.insert(f.path.clone()))
            .collect();
        EditorState { files, cwd: None }
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
fn file_multiset(files: &[FileEntry]) -> HashMap<PathBuf, Vec<(Position, Position)>> {
    let mut map: HashMap<PathBuf, Vec<(Position, Position)>> = HashMap::new();
    for f in files {
        let mut sels: Vec<_> = f.selections.iter().map(|s| (s.start, s.end)).collect();
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
        state.reorder_relative_to(&previous);
        let after = file_multiset(&state.files);
        prop_assert_eq!(before, after);
    }

    /// (b) Touched-before-unchanged partition.
    #[test]
    fn reorder_touched_before_unchanged(
        current in arb_editor_state(),
        previous in arb_editor_state(),
    ) {
        let prev_map: HashMap<&Path, &Vec<Selection>> = previous
            .files.iter().map(|f| (f.path.as_path(), &f.selections)).collect();
        let mut state = current;
        state.reorder_relative_to(&previous);

        let mut seen_unchanged = false;
        for f in &state.files {
            let is_unchanged = prev_map.get(f.path.as_path())
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
        let prev_map: HashMap<&Path, (usize, &Vec<Selection>)> = previous
            .files.iter().enumerate()
            .map(|(i, f)| (f.path.as_path(), (i, &f.selections))).collect();

        let input_order: Vec<PathBuf> = current.files.iter().map(|f| f.path.clone()).collect();
        let mut state = current;
        state.reorder_relative_to(&previous);

        // Touched files: should maintain relative input order
        let touched: Vec<&Path> = state.files.iter()
            .filter(|f| {
                prev_map.get(f.path.as_path())
                    .is_none_or(|(_, prev_sels)| *prev_sels != &f.selections)
            })
            .map(|f| f.path.as_path())
            .collect();
        let touched_input_order: Vec<&Path> = input_order.iter()
            .filter(|p| touched.contains(&p.as_path()))
            .map(|p| p.as_path())
            .collect();
        prop_assert_eq!(touched, touched_input_order);

        // Unchanged files: should be sorted by previous index
        let unchanged_prev_indices: Vec<usize> = state.files.iter()
            .filter_map(|f| {
                prev_map.get(f.path.as_path()).and_then(|(idx, prev_sels)| {
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
            cwd: None,
        };
        ed.reorder_relative_to(&state);
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
            cwd: None,
        };
        alpha.reorder_relative_to(&cached);
        let expected_paths: Vec<&Path> = cached.files.iter().map(|f| f.path.as_path()).collect();
        let actual_paths: Vec<&Path> = alpha.files.iter().map(|f| f.path.as_path()).collect();
        prop_assert_eq!(actual_paths, expected_paths);
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
            cwd: None,
        };
        state.reorder_relative_to(&current);
        for (orig, reordered) in current.files.iter().zip(state.files.iter()) {
            prop_assert_eq!(&orig.selections, &reordered.selections);
        }
    }

    /// (h) New file: selection identity.
    #[test]
    fn reorder_new_file_selections(
        current in arb_editor_state(),
    ) {
        let empty = EditorState::default();
        let original_sels: Vec<Vec<Selection>> = current.files.iter()
            .map(|f| f.selections.clone())
            .collect();
        let mut state = current;
        state.reorder_relative_to(&empty);
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
        let before_sels: HashMap<PathBuf, Vec<(Position, Position)>> = current.files.iter()
            .map(|f| (f.path.clone(), f.selections.iter()
                .map(|s| (s.start, s.end)).collect()))
            .collect();
        let mut state = current;
        state.reorder_relative_to(&previous);
        for f in &state.files {
            if let Some(orig) = before_sels.get(&f.path) {
                let mut after: Vec<_> = f.selections.iter()
                    .map(|s| (s.start, s.end)).collect();
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
        let prev_map: HashMap<&Path, &Vec<Selection>> = previous
            .files.iter().map(|f| (f.path.as_path(), &f.selections)).collect();
        let mut state = current;
        state.reorder_relative_to(&previous);

        for f in &state.files {
            if let Some(prev_sels) = prev_map.get(f.path.as_path()) {
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
        let positions = Position::from_offsets(reader, &offsets).unwrap();
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
                let result = Selection::resolve_from_reader(reader, &raw).unwrap();
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
                let result = Selection::resolve_from_reader(reader, &raw).unwrap();
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
    }
}

// ── Integration test helpers ────────────────────────────────────────────────

/// Create a file with the given content in a directory, returning its path.
fn write_temp_file(dir: &Path, name: &str, content: &str) -> PathBuf {
    let path = dir.join(name);
    fs::write(&path, content).unwrap();
    path
}

/// Construct a raw editor row (bypassing the database).
fn make_editor(path: &Path, sel: Option<(i64, i64)>) -> RawEditor {
    RawEditor {
        path: path.to_path_buf(),
        sel_start: sel.map(|(s, _)| s),
        sel_end: sel.map(|(_, e)| e),
    }
}

/// Simulate one hook invocation: build state, optionally reorder, check for changes.
///
/// Between invocations, tests should round-trip the state through [`round_trip`]
/// to match what `hook::run` does with its cache file.
fn simulate(editors: Vec<RawEditor>, cwd: &Path, previous: &EditorState) -> (EditorState, bool) {
    let mut state = EditorState::build(editors, Some(cwd), &[]).unwrap();
    state.reorder_relative_to(previous);
    let changed = *previous != state;
    (state, changed)
}

/// JSON round-trip to match what `hook::run` does with its cache file.
fn round_trip(state: &EditorState) -> EditorState {
    let json = serde_json::to_string(state).unwrap();
    serde_json::from_str(&json).unwrap()
}

// ── Integration tests: hook caching and ordering ────────────────────────────

// All tests use file content "ab\ncd\n" (6 bytes) with known byte-offset mappings:
//   offset 0 → 1:1    offset 1 → 1:2    offset 2 → 1:3
//   offset 3 → 2:1    offset 4 → 2:2    offset 5 → 2:3
const TEST_CONTENT: &str = "ab\ncd\n";

/// First invocation with no cache: files appear in alphabetical order with
/// correct line:col positions resolved from byte offsets.
#[test]
fn first_invocation_alphabetical() {
    let dir = tempfile::tempdir().unwrap();
    let c = write_temp_file(dir.path(), "c.rs", TEST_CONTENT);
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);
    let b = write_temp_file(dir.path(), "b.rs", TEST_CONTENT);

    let editors = vec![
        make_editor(&c, Some((0, 0))),
        make_editor(&a, Some((3, 3))),
        make_editor(&b, Some((1, 4))),
    ];
    let (state, changed) = simulate(editors, dir.path(), &EditorState::default());

    assert!(changed);
    // BTreeMap sorts by path → a.rs, b.rs, c.rs
    assert_eq!(state.files.len(), 3);
    assert_eq!(state.files[0].path, a);
    assert_eq!(state.files[1].path, b);
    assert_eq!(state.files[2].path, c);

    // a.rs: (3,3) → cursor at 2:1
    assert_eq!(
        state.files[0].selections[0].start,
        Position::of(2, 1).unwrap()
    );
    assert_eq!(
        state.files[0].selections[0].end,
        Position::of(2, 1).unwrap()
    );

    // b.rs: (1,4) → selection 1:2-2:2
    assert_eq!(
        state.files[1].selections[0].start,
        Position::of(1, 2).unwrap()
    );
    assert_eq!(
        state.files[1].selections[0].end,
        Position::of(2, 2).unwrap()
    );

    // c.rs: (0,0) → cursor at 1:1
    assert_eq!(
        state.files[2].selections[0].start,
        Position::of(1, 1).unwrap()
    );
    assert_eq!(
        state.files[2].selections[0].end,
        Position::of(1, 1).unwrap()
    );
}

/// Identical second invocation → cache hit: `changed == false`, state equals cached.
#[test]
fn cache_hit_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);
    let b = write_temp_file(dir.path(), "b.rs", TEST_CONTENT);

    let (state1, _) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&b, Some((3, 3)))],
        dir.path(),
        &EditorState::default(),
    );
    let cached = round_trip(&state1);

    let (state2, changed) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&b, Some((3, 3)))],
        dir.path(),
        &cached,
    );

    assert!(!changed);
    assert_eq!(state2, cached);
}

/// Moving cursor in one of three files reorders that file to the front.
#[test]
fn changed_selection_reorders_file_to_front() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);
    let b = write_temp_file(dir.path(), "b.rs", TEST_CONTENT);
    let c = write_temp_file(dir.path(), "c.rs", TEST_CONTENT);

    // All cursors at 1:1
    let (state1, _) = simulate(
        vec![
            make_editor(&a, Some((0, 0))),
            make_editor(&b, Some((0, 0))),
            make_editor(&c, Some((0, 0))),
        ],
        dir.path(),
        &EditorState::default(),
    );
    let cached = round_trip(&state1);

    // Move cursor in c.rs to 2:1
    let (state2, changed) = simulate(
        vec![
            make_editor(&a, Some((0, 0))),
            make_editor(&b, Some((0, 0))),
            make_editor(&c, Some((3, 3))),
        ],
        dir.path(),
        &cached,
    );

    assert!(changed);
    assert_eq!(state2.files[0].path, c);
    // Unchanged files retain cached order
    assert_eq!(state2.files[1].path, a);
    assert_eq!(state2.files[2].path, b);
}

/// Opening an additional file places it before unchanged files.
#[test]
fn new_file_appears_first() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);
    let b = write_temp_file(dir.path(), "b.rs", TEST_CONTENT);

    let (state1, _) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&b, Some((0, 0)))],
        dir.path(),
        &EditorState::default(),
    );
    let cached = round_trip(&state1);

    // Open c.rs
    let c = write_temp_file(dir.path(), "c.rs", TEST_CONTENT);
    let (state2, changed) = simulate(
        vec![
            make_editor(&a, Some((0, 0))),
            make_editor(&b, Some((0, 0))),
            make_editor(&c, Some((0, 0))),
        ],
        dir.path(),
        &cached,
    );

    assert!(changed);
    assert_eq!(state2.files[0].path, c);
    assert_eq!(state2.files[1].path, a);
    assert_eq!(state2.files[2].path, b);
}

/// Closing a file removes it from state and triggers `changed == true`.
#[test]
fn removed_file_disappears() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);
    let b = write_temp_file(dir.path(), "b.rs", TEST_CONTENT);
    let c = write_temp_file(dir.path(), "c.rs", TEST_CONTENT);

    let (state1, _) = simulate(
        vec![
            make_editor(&a, Some((0, 0))),
            make_editor(&b, Some((0, 0))),
            make_editor(&c, Some((0, 0))),
        ],
        dir.path(),
        &EditorState::default(),
    );
    let cached = round_trip(&state1);

    // Close b.rs
    let (state2, changed) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&c, Some((0, 0)))],
        dir.path(),
        &cached,
    );

    assert!(changed);
    assert_eq!(state2.files.len(), 2);
    // Unchanged files in cached order (a was idx 0, c was idx 2)
    assert_eq!(state2.files[0].path, a);
    assert_eq!(state2.files[1].path, c);
}

/// Five invocations simulating a real session:
/// open a+b → no-op → move cursor in b → open c → close a.
#[test]
fn multi_step_sequence() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);
    let b = write_temp_file(dir.path(), "b.rs", TEST_CONTENT);
    let c = write_temp_file(dir.path(), "c.rs", TEST_CONTENT);

    // Step 1: open a+b, no cache
    let (state1, changed1) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&b, Some((0, 0)))],
        dir.path(),
        &EditorState::default(),
    );
    assert!(changed1);
    assert_eq!(state1.files[0].path, a);
    assert_eq!(state1.files[1].path, b);
    let cached1 = round_trip(&state1);

    // Step 2: no-op (identical editors)
    let (state2, changed2) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&b, Some((0, 0)))],
        dir.path(),
        &cached1,
    );
    assert!(!changed2);
    assert_eq!(state2, cached1);
    let cached2 = round_trip(&state2);

    // Step 3: move cursor in b → b comes first
    let (state3, changed3) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&b, Some((3, 3)))],
        dir.path(),
        &cached2,
    );
    assert!(changed3);
    assert_eq!(state3.files[0].path, b);
    assert_eq!(state3.files[1].path, a);
    let cached3 = round_trip(&state3);

    // Step 4: open c → c is new, then cached order (b, a)
    let (state4, changed4) = simulate(
        vec![
            make_editor(&a, Some((0, 0))),
            make_editor(&b, Some((3, 3))),
            make_editor(&c, Some((0, 0))),
        ],
        dir.path(),
        &cached3,
    );
    assert!(changed4);
    assert_eq!(state4.files[0].path, c);
    assert_eq!(state4.files[1].path, b);
    assert_eq!(state4.files[2].path, a);
    let cached4 = round_trip(&state4);

    // Step 5: close a → remaining in cached order (c, b)
    let (state5, changed5) = simulate(
        vec![make_editor(&b, Some((3, 3))), make_editor(&c, Some((0, 0)))],
        dir.path(),
        &cached4,
    );
    assert!(changed5);
    assert_eq!(state5.files.len(), 2);
    assert_eq!(state5.files[0].path, c);
    assert_eq!(state5.files[1].path, b);
}

/// Adding a third cursor to a file with two: new cursor appears first,
/// existing cursors retain cached order.
#[test]
fn selection_level_reordering() {
    let dir = tempfile::tempdir().unwrap();
    let a = write_temp_file(dir.path(), "a.rs", TEST_CONTENT);

    // First invocation: 2 cursors at 1:1 and 2:1
    let (state1, _) = simulate(
        vec![make_editor(&a, Some((0, 0))), make_editor(&a, Some((3, 3)))],
        dir.path(),
        &EditorState::default(),
    );
    assert_eq!(state1.files[0].selections.len(), 2);
    let cached = round_trip(&state1);

    // Second invocation: add cursor at offset 1 → 1:2
    let (state2, changed) = simulate(
        vec![
            make_editor(&a, Some((0, 0))),
            make_editor(&a, Some((1, 1))),
            make_editor(&a, Some((3, 3))),
        ],
        dir.path(),
        &cached,
    );

    assert!(changed);
    assert_eq!(state2.files[0].selections.len(), 3);
    // New cursor (1:2) first
    assert_eq!(
        state2.files[0].selections[0].start,
        Position::of(1, 2).unwrap()
    );
    // Then existing in cached order: 1:1, 2:1
    assert_eq!(
        state2.files[0].selections[1].start,
        Position::of(1, 1).unwrap()
    );
    assert_eq!(
        state2.files[0].selections[2].start,
        Position::of(2, 1).unwrap()
    );
}

/// Editors outside cwd are filtered out. Changes only to outside-cwd editors
/// produce a cache hit (`changed == false`).
#[test]
fn changes_outside_cwd_invisible() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("project");
    let outside = dir.path().join("other");
    fs::create_dir_all(&cwd).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let inside = write_temp_file(&cwd, "a.rs", TEST_CONTENT);
    let out_file = write_temp_file(&outside, "b.rs", TEST_CONTENT);

    // First invocation: one inside, one outside
    let (state1, _) = simulate(
        vec![
            make_editor(&inside, Some((0, 0))),
            make_editor(&out_file, Some((0, 0))),
        ],
        &cwd,
        &EditorState::default(),
    );
    assert_eq!(state1.files.len(), 1);
    assert_eq!(state1.files[0].path, inside);
    let cached = round_trip(&state1);

    // Second invocation: change cursor on outside file only
    let (state2, changed) = simulate(
        vec![
            make_editor(&inside, Some((0, 0))),
            make_editor(&out_file, Some((3, 3))),
        ],
        &cwd,
        &cached,
    );

    assert!(!changed);
    assert_eq!(state2.files.len(), 1);
    assert_eq!(state2.files[0].path, inside);
}
