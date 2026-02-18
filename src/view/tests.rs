use super::*;
use crate::state::{Line, Position};

use proptest::prelude::*;

const SAMPLE: &str = "\
fn main() {
    greet(name);
    let x = 42;
    let y = x + 1;
    log(y);
}
";

/// Create a temp directory and write a file into it, returning (dir, path).
fn setup(name: &str, content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(&path, content).unwrap();
    (dir, path)
}

/// Render in Markers mode (deterministic, no ANSI escapes).
fn render_markers(entries: &[FileEntry], cwd: Option<&Path>) -> anyhow::Result<String> {
    render_with_mode(entries, cwd, Mode::Markers, Extent::Exact)
}

fn render_ctx(
    entries: &[FileEntry],
    cwd: Option<&Path>,
    context: Extent,
) -> anyhow::Result<String> {
    render_with_mode(entries, cwd, Mode::Markers, context)
}

#[test]
fn cursor_on_line() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(3, 9).unwrap(),
            end: Position::of(3, 9).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          3:9
                let ❘x = 42;
        ");
}

#[test]
fn multi_line_selection() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(2, 5).unwrap(),
            end: Position::of(4, 15).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          2:5-4:15
                ⟦greet(name);
                let x = 42;
                let y = x ⟧+ 1;
        ");
}

#[test]
fn single_line_partial_selection() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(3, 9).unwrap(),
            end: Position::of(3, 15).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          3:9-3:15
                let ⟦x = 42⟧;
        ");
}

#[test]
fn multiple_selections_one_file() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![
            Selection {
                start: Position::of(1, 1).unwrap(),
                end: Position::of(1, 1).unwrap(),
            },
            Selection {
                start: Position::of(3, 5).unwrap(),
                end: Position::of(3, 8).unwrap(),
            },
        ],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          1:1
            ❘fn main() {
          3:5-3:8
                ⟦let⟧ x = 42;
        ");
}

#[test]
fn multiple_files() {
    let dir = tempfile::tempdir().unwrap();
    let p1 = dir.path().join("one.rs");
    let p2 = dir.path().join("two.rs");
    std::fs::write(&p1, "line one\nline two\n").unwrap();
    std::fs::write(&p2, "alpha\nbeta\ngamma\n").unwrap();
    let entries = vec![
        FileEntry {
            path: p1,
            selections: vec![Selection {
                start: Position::of(1, 5).unwrap(),
                end: Position::of(1, 5).unwrap(),
            }],
        },
        FileEntry {
            path: p2,
            selections: vec![Selection {
                start: Position::of(2, 1).unwrap(),
                end: Position::of(3, 6).unwrap(),
            }],
        },
    ];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        one.rs
          1:5
            line❘ one

        two.rs
          2:1-3:6
            ⟦beta
            gamma⟧
        ");
}

#[test]
fn selection_at_line_start() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(1, 1).unwrap(),
            end: Position::of(2, 5).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          1:1-2:5
            ⟦fn main() {
                ⟧greet(name);
        ");
}

#[test]
fn selection_at_line_end() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(1, 12).unwrap(),
            end: Position::of(1, 12).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          1:12
            fn main() {❘
        ");
}

#[test]
fn color_mode_cursor() {
    let (dir, path) = setup("main.rs", "hello\n");
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(1, 3).unwrap(),
            end: Position::of(1, 3).unwrap(),
        }],
    }];
    let result = render_with_mode(&entries, Some(dir.path()), Mode::Color, Extent::Exact).unwrap();
    // Bold path, dim position, inverse cursor char
    assert!(result.contains(ansi::BOLD));
    assert!(result.contains(ansi::DIM));
    assert!(result.contains(ansi::INVERSE));
    assert!(result.contains("he"));
    assert!(result.contains("lo"));
}

#[test]
fn color_mode_selection() {
    let (dir, path) = setup("main.rs", "hello world\n");
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(1, 3).unwrap(),
            end: Position::of(1, 8).unwrap(),
        }],
    }];
    let result = render_with_mode(&entries, Some(dir.path()), Mode::Color, Extent::Exact).unwrap();
    // Inverse around the selected text
    assert!(result.contains(&format!("{}llo w{}", ansi::INVERSE, ansi::RESET)));
}

#[test]
fn parse_display_cursor() {
    let sel: Selection = "5:12".parse().unwrap();
    assert_eq!(sel.start, Position::of(5, 12).unwrap());
    assert_eq!(sel.end, Position::of(5, 12).unwrap());
}

#[test]
fn parse_display_range() {
    let sel: Selection = "19:40-24:6".parse().unwrap();
    assert_eq!(sel.start, Position::of(19, 40).unwrap());
    assert_eq!(sel.end, Position::of(24, 6).unwrap());
}

#[test]
fn parse_display_roundtrip() {
    let original = Selection {
        start: Position::of(10, 5).unwrap(),
        end: Position::of(20, 15).unwrap(),
    };
    let display = original.to_string();
    let parsed: Selection = display.parse().unwrap();
    assert_eq!(parsed, original);

    let cursor = Selection {
        start: Position::of(3, 7).unwrap(),
        end: Position::of(3, 7).unwrap(),
    };
    let display = cursor.to_string();
    let parsed: Selection = display.parse().unwrap();
    assert_eq!(parsed, cursor);
}

#[test]
fn parse_compact_basic() {
    let entries = parse_compact("src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].path.to_str().unwrap(), "src/foo.rs");
    assert_eq!(entries[0].selections.len(), 2);
    assert_eq!(entries[1].path.to_str().unwrap(), "src/bar.rs");
    assert_eq!(entries[1].selections.len(), 1);
}

#[test]
fn parse_compact_with_commas() {
    let entries = parse_compact("src/foo.rs 5:12, 10:3 src/bar.rs 1:1").unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].selections.len(), 2);
    assert_eq!(entries[1].selections.len(), 1);
}

#[test]
fn parse_compact_quoted_path() {
    let entries = parse_compact(r#""path with spaces/foo.rs" 5:12"#).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.to_str().unwrap(), "path with spaces/foo.rs");
    assert_eq!(entries[0].selections.len(), 1);
}

#[test]
fn parse_compact_concatenation_heuristic() {
    let entries = parse_compact("path with spaces/foo.rs 5:12").unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].path.to_str().unwrap(), "path with spaces/foo.rs");
    assert_eq!(entries[0].selections.len(), 1);
}

#[test]
fn render_with_cwd() {
    let (dir, path) = setup("test.rs", "hello\nworld\n");
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(1, 3).unwrap(),
            end: Position::of(1, 3).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        test.rs
          1:3
            he❘llo
        ");
}

#[test]
fn context_before_after() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(3, 9).unwrap(),
            end: Position::of(3, 9).unwrap(),
        }],
    }];
    let ctx = Extent::Lines {
        before: 1,
        after: 1,
    };
    let result = render_ctx(&entries, Some(dir.path()), ctx).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          3:9
                greet(name);
                let ❘x = 42;
                let y = x + 1;
        ");
}

#[test]
fn context_clamps_to_file_bounds() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(1, 1).unwrap(),
            end: Position::of(1, 1).unwrap(),
        }],
    }];
    let ctx = Extent::Lines {
        before: 5,
        after: 1,
    };
    let result = render_ctx(&entries, Some(dir.path()), ctx).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          1:1
            ❘fn main() {
                greet(name);
        ");
}

#[test]
fn context_around_selection() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(3, 5).unwrap(),
            end: Position::of(4, 9).unwrap(),
        }],
    }];
    let ctx = Extent::Lines {
        before: 1,
        after: 1,
    };
    let result = render_ctx(&entries, Some(dir.path()), ctx).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          3:5-4:9
                greet(name);
                ⟦let x = 42;
                let ⟧y = x + 1;
                log(y);
        ");
}

#[test]
fn merged_overlapping_contexts() {
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![
            Selection {
                start: Position::of(2, 5).unwrap(),
                end: Position::of(2, 5).unwrap(),
            },
            Selection {
                start: Position::of(4, 9).unwrap(),
                end: Position::of(4, 9).unwrap(),
            },
        ],
    }];
    // With 1 line of context, lines 1-3 and 3-5 overlap → merged into one group
    let ctx = Extent::Lines {
        before: 1,
        after: 1,
    };
    let result = render_ctx(&entries, Some(dir.path()), ctx).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          2:5, 4:9
            fn main() {
                ❘greet(name);
                let x = 42;
                let ❘y = x + 1;
                log(y);
        ");
}

#[test]
fn separate_non_overlapping_contexts() {
    // 6-line file, cursors at 1 and 6 with 0 context: two separate groups
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![
            Selection {
                start: Position::of(1, 1).unwrap(),
                end: Position::of(1, 1).unwrap(),
            },
            Selection {
                start: Position::of(6, 1).unwrap(),
                end: Position::of(6, 1).unwrap(),
            },
        ],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    insta::assert_snapshot!(result, @r"
        main.rs
          1:1
            ❘fn main() {
          6:1
            ❘}
        ");
}

#[test]
fn cursor_like_display() {
    // Single-char selection (start.col + 1 == end.col) should display as cursor
    let (dir, path) = setup("main.rs", SAMPLE);
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(3, 9).unwrap(),
            end: Position::of(3, 10).unwrap(),
        }],
    }];
    let result = render_markers(&entries, Some(dir.path())).unwrap();
    // Header shows "3:9" not "3:9-3:10"
    insta::assert_snapshot!(result, @r"
        main.rs
          3:9
                let ❘x = 42;
        ");
}

#[test]
fn full_file_cursor() {
    let (dir, path) = setup("small.rs", "aaa\nbbb\nccc\n");
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(2, 2).unwrap(),
            end: Position::of(2, 2).unwrap(),
        }],
    }];
    let result = render_ctx(&entries, Some(dir.path()), Extent::Full).unwrap();
    insta::assert_snapshot!(result, @r"
        small.rs
            aaa
            b❘bb
            ccc
        ");
}

#[test]
fn full_file_selection() {
    let (dir, path) = setup("small.rs", "aaa\nbbb\nccc\nddd\n");
    let entries = vec![FileEntry {
        path,
        selections: vec![Selection {
            start: Position::of(2, 2).unwrap(),
            end: Position::of(3, 3).unwrap(),
        }],
    }];
    let result = render_ctx(&entries, Some(dir.path()), Extent::Full).unwrap();
    insta::assert_snapshot!(result, @r"
        small.rs
            aaa
            b⟦bb
            cc⟧c
            ddd
        ");
}

// ---------------------------------------------------------------------------
// Proptest helpers
// ---------------------------------------------------------------------------

/// Strip ❘⟦⟧ from a string, preserving all other content.
fn strip_markers(s: &str) -> String {
    s.chars()
        .filter(|&c| c != CURSOR && c != SEL_OPEN && c != SEL_CLOSE)
        .collect()
}

/// Strip ANSI escape sequences (ESC[...m) from a string.
#[allow(dead_code)]
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            // Skip until 'm'
            while let Some(&nc) = chars.peek() {
                chars.next();
                if nc == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Write a file into a tempdir and build a single FileEntry.
fn setup_entry(
    dir: &tempfile::TempDir,
    content: &str,
    sels: Vec<Selection>,
) -> (std::path::PathBuf, Vec<FileEntry>) {
    let path = dir.path().join("test.txt");
    std::fs::write(&path, content).unwrap();
    let entries = vec![FileEntry {
        path: path.clone(),
        selections: sels,
    }];
    (path, entries)
}

// ---------------------------------------------------------------------------
// Proptest strategies
// ---------------------------------------------------------------------------

fn arb_position() -> impl Strategy<Value = Position> {
    (1usize..20, 1usize..50).prop_map(|(line, col)| Position::of(line, col).unwrap())
}

fn arb_selection() -> impl Strategy<Value = Selection> {
    (arb_position(), arb_position()).prop_map(|(start, end)| Selection { start, end })
}

fn arb_cursor_like() -> impl Strategy<Value = Selection> {
    prop_oneof![
        arb_position().prop_map(|p| Selection { start: p, end: p }),
        arb_position().prop_map(|p| {
            let end = Position::of(p.line.get(), p.col.get() + 1).unwrap();
            Selection { start: p, end }
        }),
    ]
}

fn arb_range_selection() -> impl Strategy<Value = Selection> {
    arb_selection().prop_filter("must not be cursor-like", |s| !s.is_cursor_like())
}

fn arb_extent() -> impl Strategy<Value = Extent> {
    prop_oneof![
        Just(Extent::Exact),
        (0usize..10, 0usize..10)
            .prop_map(|(before, after)| Extent::Lines { before, after }),
        Just(Extent::Full),
    ]
}

fn arb_file_content() -> impl Strategy<Value = String> {
    proptest::collection::vec("[a-zA-Z0-9 _=+;(){}]{0,80}", 1..30)
        .prop_map(|lines| lines.join("\n") + "\n")
}

fn arb_file_with_selections() -> impl Strategy<Value = (String, Vec<Selection>)> {
    arb_file_content().prop_flat_map(|content| {
        let line_count = content.lines().count().max(1);
        let longest = content.lines().map(|l| l.len()).max().unwrap_or(1).max(1);
        let in_bounds = (1..=line_count, 1..=longest + 1)
            .prop_map(|(l, c)| Position::of(l, c).unwrap());
        let in_bounds_sel = (in_bounds.clone(), in_bounds)
            .prop_map(|(start, end)| Selection { start, end });
        let sels = proptest::collection::vec(in_bounds_sel, 1..6);
        (Just(content), sels)
    })
}

fn arb_position_token() -> impl Strategy<Value = String> {
    prop_oneof![
        (1usize..100, 1usize..100).prop_map(|(l, c)| format!("{l}:{c}")),
        (1usize..100, 1usize..100, 1usize..100, 1usize..100)
            .prop_map(|(l1, c1, l2, c2)| format!("{l1}:{c1}-{l2}:{c2}")),
    ]
}

fn arb_fuzz_string() -> impl Strategy<Value = String> {
    "[ -~]{0,200}"
}

// ---------------------------------------------------------------------------
// Block 1 — parse_compact
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn parse_compact_no_panic(input in arb_fuzz_string()) {
        let _ = parse_compact(&input);
    }

    #[test]
    fn parse_compact_round_trip(
        paths in proptest::collection::vec("[a-z]{1,8}/[a-z]{1,8}\\.[a-z]{1,3}", 1..4),
        sels_per_file in proptest::collection::vec(
            proptest::collection::vec(arb_selection(), 1..4),
            1..4,
        ),
    ) {
        // Build FileEntry list from generated paths and selections.
        let count = paths.len().min(sels_per_file.len());
        let entries: Vec<FileEntry> = (0..count)
            .map(|i| FileEntry {
                path: paths[i].clone().into(),
                selections: sels_per_file[i].clone(),
            })
            .collect();

        // Display → parse round-trip
        let display: String = entries
            .iter()
            .map(|e| {
                let sels: Vec<String> = e.selections.iter().map(|s| s.to_string()).collect();
                format!("{} {}", e.path.display(), sels.join(" "))
            })
            .collect::<Vec<_>>()
            .join(" ");

        let parsed = parse_compact(&display).unwrap();
        prop_assert_eq!(parsed.len(), entries.len());
        for (orig, got) in entries.iter().zip(parsed.iter()) {
            prop_assert_eq!(&orig.path, &got.path);
            prop_assert_eq!(&orig.selections, &got.selections);
        }
    }

    #[test]
    fn position_tokens_recognized(
        path in "[a-z]{1,5}/[a-z]{1,5}\\.[a-z]{1,3}",
        token in arb_position_token(),
    ) {
        let input = format!("{path} {token}");
        let entries = parse_compact(&input).unwrap();
        prop_assert!(!entries.is_empty(), "should parse at least one entry");
        prop_assert!(
            !entries[0].selections.is_empty(),
            "token {token} should be recognized as a selection"
        );
    }
}

// ---------------------------------------------------------------------------
// Block 2 — compute_groups
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn groups_partition_all_selections(
        sels in proptest::collection::vec(arb_selection(), 1..10),
        extra_lines in 0usize..30,
        extent in arb_extent(),
    ) {
        let max_line = sels.iter().flat_map(|s| [s.start.line.get(), s.end.line.get()]).max().unwrap_or(1);
        let total_lines = max_line + extra_lines;
        let groups = Group::compute(&sels, total_lines, extent);
        let mut seen: Vec<*const Selection> = Vec::new();
        for g in &groups {
            for &s in &g.sels {
                seen.push(s as *const Selection);
            }
        }
        // Every input selection should appear exactly once (by pointer).
        for sel in &sels {
            let ptr = sel as *const Selection;
            let count = seen.iter().filter(|&&p| p == ptr).count();
            prop_assert_eq!(count, 1, "selection {:?} appeared {} times", sel, count);
        }
    }

    #[test]
    fn groups_sorted_by_first_line(
        sels in proptest::collection::vec(arb_selection(), 1..10),
        extra_lines in 0usize..30,
        extent in arb_extent(),
    ) {
        let max_line = sels.iter().flat_map(|s| [s.start.line.get(), s.end.line.get()]).max().unwrap_or(1);
        let total_lines = max_line + extra_lines;
        let groups = Group::compute(&sels, total_lines, extent);
        for w in groups.windows(2) {
            prop_assert!(
                w[0].first_line <= w[1].first_line,
                "groups not sorted: {} > {}",
                w[0].first_line,
                w[1].first_line
            );
        }
    }

    #[test]
    fn groups_maximally_merged(
        sels in proptest::collection::vec(arb_selection(), 1..10),
        extra_lines in 0usize..30,
        extent in arb_extent(),
    ) {
        let max_line = sels.iter().flat_map(|s| [s.start.line.get(), s.end.line.get()]).max().unwrap_or(1);
        let total_lines = max_line + extra_lines;
        let groups = Group::compute(&sels, total_lines, extent);
        for w in groups.windows(2) {
            prop_assert!(
                w[1].first_line > w[0].last_line.saturating_add(1),
                "consecutive groups should have gap >= 2 lines, got {} and {}",
                w[0].last_line,
                w[1].first_line
            );
        }
    }

    #[test]
    fn groups_valid_ranges(
        sels in proptest::collection::vec(arb_selection(), 1..10),
        extra_lines in 0usize..30,
        extent in arb_extent(),
    ) {
        let max_line = sels.iter().flat_map(|s| [s.start.line.get(), s.end.line.get()]).max().unwrap_or(1);
        let total_lines = max_line + extra_lines;
        let groups = Group::compute(&sels, total_lines, extent);
        for g in &groups {
            prop_assert!(
                g.first_line <= g.last_line,
                "first > last: {} > {}",
                g.first_line,
                g.last_line
            );
        }
    }

    #[test]
    fn group_covers_its_selections(
        sels in proptest::collection::vec(arb_selection(), 1..10),
        extra_lines in 0usize..30,
        extent in arb_extent(),
    ) {
        let max_line = sels.iter().flat_map(|s| [s.start.line.get(), s.end.line.get()]).max().unwrap_or(1);
        let total_lines = max_line + extra_lines;
        let total = Line::new(total_lines).unwrap();
        let (ctx_b, ctx_a) = match extent {
            Extent::Exact => (0, 0),
            Extent::Lines { before, after } => (before, after),
            Extent::Full => (total_lines, total_lines),
        };
        let groups = Group::compute(&sels, total_lines, extent);
        for g in &groups {
            for sel in &g.sels {
                let (first, last) = if sel.is_cursor_like() {
                    (sel.start.line, sel.start.line)
                } else {
                    (sel.start.line.min(sel.end.line), sel.start.line.max(sel.end.line))
                };
                let vis_first = first.saturating_sub(ctx_b);
                let vis_last = last.saturating_add(ctx_a).min(total);
                // The visible range must overlap with the group range.
                prop_assert!(
                    vis_first <= g.last_line && vis_last >= g.first_line,
                    "selection {:?} vis [{}, {}] doesn't overlap group [{}, {}]",
                    sel,
                    vis_first,
                    vis_last,
                    g.first_line,
                    g.last_line
                );
            }
        }
    }

    #[test]
    fn full_extent_single_group(
        sels in proptest::collection::vec(arb_selection(), 1..10),
        extra_lines in 0usize..30,
    ) {
        let max_line = sels.iter().flat_map(|s| [s.start.line.get(), s.end.line.get()]).max().unwrap_or(1);
        let total_lines = max_line + extra_lines;
        let groups = Group::compute(&sels, total_lines, Extent::Full);
        prop_assert_eq!(groups.len(), 1, "Full extent should yield exactly 1 group");
    }

    #[test]
    fn empty_selections_empty_groups(
        total_lines in 1usize..50,
        extent in arb_extent(),
    ) {
        let sels: Vec<Selection> = vec![];
        let groups = Group::compute(&sels, total_lines, extent);
        prop_assert!(groups.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Block 3 — display_sel / is_cursor_like
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn display_sel_cursor_like_no_dash(sel in arb_cursor_like()) {
        let d = sel.display_header();
        prop_assert!(!d.contains('-'), "cursor-like display should not contain dash: {}", d);
    }

    #[test]
    fn display_sel_range_has_dash(sel in arb_range_selection()) {
        let d = sel.display_header();
        prop_assert!(d.contains('-'), "range display should contain dash: {}", d);
    }

    #[test]
    fn cursor_is_cursor_like(pos in arb_position()) {
        let sel = Selection { start: pos, end: pos };
        prop_assert!(sel.is_cursor_like());
    }

    #[test]
    fn single_char_is_cursor_like(pos in arb_position()) {
        let end = Position::of(pos.line.get(), pos.col.get() + 1).unwrap();
        let sel = Selection { start: pos, end };
        prop_assert!(sel.is_cursor_like());
    }

    #[test]
    fn range_not_cursor_like(sel in arb_range_selection()) {
        prop_assert!(!sel.is_cursor_like());
    }
}

// ---------------------------------------------------------------------------
// Block 4 — render_with_mode
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn render_never_panics(
        (content, sels) in arb_file_with_selections(),
        mode in prop_oneof![Just(Mode::Markers), Just(Mode::Color)],
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let _ = render_with_mode(&entries, Some(dir.path()), mode, extent);
    }

    #[test]
    fn markers_mode_no_ansi(
        (content, sels) in arb_file_with_selections(),
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, extent).unwrap();
        prop_assert!(
            !result.contains('\x1b'),
            "Markers output must not contain ANSI escapes"
        );
    }

    #[test]
    fn color_mode_no_unicode_markers(
        (content, sels) in arb_file_with_selections(),
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Color, extent).unwrap();
        prop_assert!(!result.contains(CURSOR), "Color output must not contain ❘");
        prop_assert!(!result.contains(SEL_OPEN), "Color output must not contain ⟦");
        prop_assert!(!result.contains(SEL_CLOSE), "Color output must not contain ⟧");
    }

    #[test]
    fn output_ends_with_newline(
        (content, sels) in arb_file_with_selections(),
        mode in prop_oneof![Just(Mode::Markers), Just(Mode::Color)],
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), mode, extent).unwrap();
        if !result.is_empty() {
            prop_assert!(result.ends_with('\n'), "non-empty output must end with newline");
        }
    }

    #[test]
    fn output_line_indent_structure(
        (content, sels) in arb_file_with_selections(),
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, extent).unwrap();
        for line in result.lines() {
            let is_empty = line.is_empty();
            let is_path = !line.starts_with(' '); // unindented
            let is_header = line.starts_with("  ") && !line.starts_with("    ");
            let is_content = line.starts_with("    ");
            prop_assert!(
                is_empty || is_path || is_header || is_content,
                "unexpected indent structure: {:?}",
                line
            );
        }
    }

    #[test]
    fn modes_structural_equivalence(
        (content, sels) in arb_file_with_selections(),
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let markers = render_with_mode(&entries, Some(dir.path()), Mode::Markers, extent).unwrap();
        let color = render_with_mode(&entries, Some(dir.path()), Mode::Color, extent).unwrap();
        let m_lines = markers.lines().count();
        let c_lines = color.lines().count();
        prop_assert_eq!(
            m_lines, c_lines,
            "Markers ({}) and Color ({}) should have same line count", m_lines, c_lines
        );
    }
}

// ---------------------------------------------------------------------------
// Block 5 — content-level rendering oracles
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(200))]

    #[test]
    fn full_mode_content_preservation(
        (content, sels) in arb_file_with_selections(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Full).unwrap();
        let file_lines: Vec<&str> = content.lines().collect();
        let content_lines: Vec<&str> = result
            .lines()
            .filter(|l| l.starts_with("    "))
            .collect();
        prop_assert_eq!(
            content_lines.len(),
            file_lines.len(),
            "content line count mismatch: got {} expected {}",
            content_lines.len(),
            file_lines.len()
        );
        for (i, (rendered, original)) in content_lines.iter().zip(file_lines.iter()).enumerate() {
            let stripped = strip_markers(&rendered[4..]); // remove 4-space indent
            // The renderer adds a space on empty lines inside a selection
            // to visually connect the highlighted region.
            let expected = if original.is_empty() && stripped == " " {
                " "
            } else {
                *original
            };
            prop_assert_eq!(
                stripped.as_str(),
                expected,
                "line {} content mismatch:\n  rendered: {:?}\n  stripped: {:?}\n  original: {:?}",
                i + 1,
                rendered,
                stripped,
                original
            );
        }
    }

    #[test]
    fn markers_balanced_and_nested(
        (content, sels) in arb_file_with_selections(),
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, extent).unwrap();
        let mut depth: i64 = 0;
        for c in result.chars() {
            if c == SEL_OPEN {
                depth += 1;
            } else if c == SEL_CLOSE {
                depth -= 1;
            }
            prop_assert!(depth >= 0, "bracket depth went negative");
        }
        prop_assert_eq!(depth, 0, "unbalanced brackets: final depth {}", depth);
    }

    #[test]
    fn cursor_column_oracle(
        content in arb_file_content(),
        line_idx in 0usize..29,
        col_frac in 0.0f64..1.0,
    ) {
        let file_lines: Vec<&str> = content.lines().collect();
        if file_lines.is_empty() {
            return Ok(());
        }
        let line_num = (line_idx % file_lines.len()) + 1;
        let line = file_lines[line_num - 1];
        if line.is_empty() {
            return Ok(());
        }
        let col = (col_frac * line.len() as f64).floor() as usize + 1;
        let col = col.min(line.len());

        let sel = Selection {
            start: Position::of(line_num, col).unwrap(),
            end: Position::of(line_num, col).unwrap(),
        };
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, vec![sel]);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Exact).unwrap();

        // Find the content line (4-space indent, after header lines)
        let content_line = result.lines().find(|l| l.starts_with("    "));
        prop_assert!(content_line.is_some(), "should have a content line");
        let rendered = &content_line.unwrap()[4..];
        let expected = format!(
            "{}{}{}",
            &line[..col - 1],
            CURSOR,
            &line[col - 1..]
        );
        prop_assert_eq!(rendered, expected.as_str(), "cursor oracle mismatch");
    }

    #[test]
    fn single_line_selection_oracle(
        content in arb_file_content(),
        line_idx in 0usize..29,
        col_frac1 in 0.0f64..1.0,
        col_frac2 in 0.0f64..1.0,
    ) {
        let file_lines: Vec<&str> = content.lines().collect();
        if file_lines.is_empty() {
            return Ok(());
        }
        let line_num = (line_idx % file_lines.len()) + 1;
        let line = file_lines[line_num - 1];
        if line.len() < 2 {
            return Ok(());
        }
        let mut c1 = (col_frac1 * line.len() as f64).floor() as usize + 1;
        let mut c2 = (col_frac2 * line.len() as f64).floor() as usize + 1;
        c1 = c1.min(line.len());
        c2 = c2.min(line.len());
        if c1 == c2 {
            return Ok(());
        }
        if c1 > c2 {
            std::mem::swap(&mut c1, &mut c2);
        }
        // Ensure it's not cursor-like
        if c2 == c1 + 1 {
            return Ok(());
        }

        let sel = Selection {
            start: Position::of(line_num, c1).unwrap(),
            end: Position::of(line_num, c2).unwrap(),
        };
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, vec![sel]);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Exact).unwrap();

        let content_line = result.lines().find(|l| l.starts_with("    "));
        prop_assert!(content_line.is_some(), "should have a content line");
        let rendered = &content_line.unwrap()[4..];
        let expected = format!(
            "{}{}{}{}{}",
            &line[..c1 - 1],
            SEL_OPEN,
            &line[c1 - 1..c2 - 1],
            SEL_CLOSE,
            &line[c2 - 1..]
        );
        prop_assert_eq!(rendered, expected.as_str(), "selection oracle mismatch");
    }

    #[test]
    fn color_mode_no_escape_leak(
        (content, sels) in arb_file_with_selections(),
        extent in arb_extent(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Color, extent).unwrap();
        for line in result.lines() {
            // Count INVERSE/RESET pairs to check depth ends at 0
            let mut depth: i64 = 0;
            let stripped = line.to_string();
            let mut i = 0;
            let bytes = stripped.as_bytes();
            while i < bytes.len() {
                if bytes[i] == b'\x1b' && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                    // Find the 'm'
                    let start = i;
                    while i < bytes.len() && bytes[i] != b'm' {
                        i += 1;
                    }
                    if i < bytes.len() {
                        let seq = &stripped[start..=i];
                        if seq == ansi::INVERSE {
                            depth += 1;
                        } else if seq == ansi::RESET {
                            depth = 0; // RESET clears all
                        }
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            prop_assert_eq!(
                depth, 0,
                "ANSI state leaked at end of line: {:?}",
                line
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Block 6 — line_events
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    #[test]
    fn line_events_sorted(
        sels in proptest::collection::vec(arb_selection(), 1..6),
        line_num in 1usize..20,
    ) {
        let sel_refs: Vec<&Selection> = sels.iter().collect();
        let (events, _) = line_events(&sel_refs, Line::new(line_num).unwrap());
        for w in events.windows(2) {
            prop_assert!(w[0] <= w[1], "events not sorted: {:?}", events);
        }
    }

    #[test]
    fn line_events_empty_for_distant_line(
        sels in proptest::collection::vec(
            (1usize..5, 1usize..50, 1usize..5, 1usize..50).prop_map(
                |(l1, c1, l2, c2)| Selection {
                    start: Position::of(l1, c1).unwrap(),
                    end: Position::of(l2, c2).unwrap(),
                }
            ),
            1..6,
        ),
    ) {
        let sel_refs: Vec<&Selection> = sels.iter().collect();
        let (events, in_sel) = line_events(&sel_refs, Line::new(100).unwrap());
        prop_assert!(events.is_empty(), "should have no events for distant line");
        prop_assert!(!in_sel, "should not be in selection at distant line");
    }
}

// ---------------------------------------------------------------------------
// Block 7 — cross-cutting
// ---------------------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100))]

    #[test]
    fn markers_cursor_count(
        (content, sels) in arb_file_with_selections(),
    ) {
        let file_lines: Vec<&str> = content.lines().collect();
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels.clone());
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Full).unwrap();

        let expected_cursors = sels
            .iter()
            .filter(|s| s.is_cursor_like() && s.start.line.get() <= file_lines.len())
            .count();
        let actual_cursors = result.chars().filter(|&c| c == CURSOR).count();
        prop_assert_eq!(
            actual_cursors,
            expected_cursors,
            "cursor count mismatch: got {}, expected {}", actual_cursors, expected_cursors
        );
    }

    #[test]
    fn markers_bracket_pair_count(
        (content, sels) in arb_file_with_selections(),
    ) {
        let file_lines: Vec<&str> = content.lines().collect();
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels.clone());
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Full).unwrap();

        let expected_brackets = sels
            .iter()
            .filter(|s| {
                if s.is_cursor_like() {
                    return false;
                }
                let first = s.start.line.min(s.end.line);
                let last = s.start.line.max(s.end.line);
                let total = file_lines.len();
                first.get() <= total || last.get() <= total
            })
            .count();
        let actual_open = result.chars().filter(|&c| c == SEL_OPEN).count();
        prop_assert_eq!(
            actual_open,
            expected_brackets,
            "bracket pair count mismatch: got {}, expected {}", actual_open, expected_brackets
        );
    }

    #[test]
    fn full_extent_no_headers(
        (content, sels) in arb_file_with_selections(),
    ) {
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Full).unwrap();
        for line in result.lines() {
            let is_header = line.starts_with("  ") && !line.starts_with("    ");
            prop_assert!(
                !is_header,
                "Full extent should not have header lines, got: {:?}",
                line
            );
        }
    }

    #[test]
    fn full_extent_line_count(
        (content, sels) in arb_file_with_selections(),
    ) {
        let file_line_count = content.lines().count();
        let dir = tempfile::tempdir().unwrap();
        let (_path, entries) = setup_entry(&dir, &content, sels);
        let result = render_with_mode(&entries, Some(dir.path()), Mode::Markers, Extent::Full).unwrap();
        let content_lines = result.lines().filter(|l| l.starts_with("    ")).count();
        prop_assert_eq!(
            content_lines,
            file_line_count,
            "Full extent should render every file line exactly once"
        );
    }
}
