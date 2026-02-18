use super::*;
use crate::state::Position;

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
            start: Position { line: 3, col: 9 },
            end: Position { line: 3, col: 9 },
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
            start: Position { line: 2, col: 5 },
            end: Position { line: 4, col: 15 },
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
            start: Position { line: 3, col: 9 },
            end: Position { line: 3, col: 15 },
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
                start: Position { line: 1, col: 1 },
                end: Position { line: 1, col: 1 },
            },
            Selection {
                start: Position { line: 3, col: 5 },
                end: Position { line: 3, col: 8 },
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
                start: Position { line: 1, col: 5 },
                end: Position { line: 1, col: 5 },
            }],
        },
        FileEntry {
            path: p2,
            selections: vec![Selection {
                start: Position { line: 2, col: 1 },
                end: Position { line: 3, col: 6 },
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
            start: Position { line: 1, col: 1 },
            end: Position { line: 2, col: 5 },
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
            start: Position { line: 1, col: 12 },
            end: Position { line: 1, col: 12 },
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
            start: Position { line: 1, col: 3 },
            end: Position { line: 1, col: 3 },
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
            start: Position { line: 1, col: 3 },
            end: Position { line: 1, col: 8 },
        }],
    }];
    let result = render_with_mode(&entries, Some(dir.path()), Mode::Color, Extent::Exact).unwrap();
    // Inverse around the selected text
    assert!(result.contains(&format!("{}llo w{}", ansi::INVERSE, ansi::RESET)));
}

#[test]
fn parse_display_cursor() {
    let sel = Selection::parse_display("5:12").unwrap();
    assert_eq!(sel.start, Position { line: 5, col: 12 });
    assert_eq!(sel.end, Position { line: 5, col: 12 });
}

#[test]
fn parse_display_range() {
    let sel = Selection::parse_display("19:40-24:6").unwrap();
    assert_eq!(sel.start, Position { line: 19, col: 40 });
    assert_eq!(sel.end, Position { line: 24, col: 6 });
}

#[test]
fn parse_display_roundtrip() {
    let original = Selection {
        start: Position { line: 10, col: 5 },
        end: Position { line: 20, col: 15 },
    };
    let display = original.to_string();
    let parsed = Selection::parse_display(&display).unwrap();
    assert_eq!(parsed, original);

    let cursor = Selection {
        start: Position { line: 3, col: 7 },
        end: Position { line: 3, col: 7 },
    };
    let display = cursor.to_string();
    let parsed = Selection::parse_display(&display).unwrap();
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
            start: Position { line: 1, col: 3 },
            end: Position { line: 1, col: 3 },
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
            start: Position { line: 3, col: 9 },
            end: Position { line: 3, col: 9 },
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
            start: Position { line: 1, col: 1 },
            end: Position { line: 1, col: 1 },
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
            start: Position { line: 3, col: 5 },
            end: Position { line: 4, col: 9 },
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
                start: Position { line: 2, col: 5 },
                end: Position { line: 2, col: 5 },
            },
            Selection {
                start: Position { line: 4, col: 9 },
                end: Position { line: 4, col: 9 },
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
                start: Position { line: 1, col: 1 },
                end: Position { line: 1, col: 1 },
            },
            Selection {
                start: Position { line: 6, col: 1 },
                end: Position { line: 6, col: 1 },
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
            start: Position { line: 3, col: 9 },
            end: Position { line: 3, col: 10 },
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
            start: Position { line: 2, col: 2 },
            end: Position { line: 2, col: 2 },
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
            start: Position { line: 2, col: 2 },
            end: Position { line: 3, col: 3 },
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
