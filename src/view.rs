use std::io::IsTerminal;
use std::path::Path;

use anyhow::Context;

use crate::state::resolve::relativize;
use crate::state::{FileEntry, Selection};

/// Cursor marker: U+2758 Light Vertical Bar (non-TTY).
const CURSOR: char = '❘';
/// Selection start marker: U+27E6 Mathematical Left White Square Bracket (non-TTY).
const SEL_OPEN: char = '⟦';
/// Selection end marker: U+27E7 Mathematical Right White Square Bracket (non-TTY).
const SEL_CLOSE: char = '⟧';

/// ANSI escape sequences for TTY color mode.
mod ansi {
    pub const BOLD: &str = "\x1b[1m";
    pub const DIM: &str = "\x1b[2m";
    pub const INVERSE: &str = "\x1b[7m";
    pub const RESET: &str = "\x1b[0m";
}

/// Whether to use ANSI colors or Unicode markers.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Color,
    Markers,
}

impl Mode {
    fn detect() -> Self {
        if std::env::var_os("NO_COLOR").is_some() {
            return Mode::Markers;
        }
        if std::io::stdout().is_terminal() {
            Mode::Color
        } else {
            Mode::Markers
        }
    }
}

/// How much file content to show around each selection.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Extent {
    /// Only the lines spanned by the selection/cursor.
    Exact,
    /// Additional context lines before/after.
    Lines { before: usize, after: usize },
    /// Entire file contents.
    Full,
}

/// Parse compact format into FileEntry list.
///
/// Tokens matching `\d+:\d+(-\d+:\d+)?` (with optional trailing comma) are
/// positions; everything else starts a new file path. Terminal markers (`$`)
/// are skipped.
pub fn parse_compact(input: &str) -> anyhow::Result<Vec<FileEntry>> {
    let tokens = tokenize(input);
    let mut entries: Vec<FileEntry> = Vec::new();
    let mut current_path: Option<String> = None;
    let mut current_sels: Vec<Selection> = Vec::new();

    for token in tokens {
        // Skip terminal markers
        if token == "$" {
            // Flush current file without its last "path" component (which was
            // actually the terminal cwd). The `$` means the preceding path was
            // a terminal, not a file.
            if current_path.is_some() {
                // The accumulated path was a terminal dir — discard it.
                current_path = None;
                current_sels.clear();
            }
            continue;
        }

        // Strip trailing comma for position detection
        let stripped = token.strip_suffix(',').unwrap_or(&token);

        if is_position(stripped) {
            let sel = Selection::parse_display(stripped)
                .with_context(|| format!("bad position: {stripped}"))?;
            current_sels.push(sel);
        } else {
            // New path token. Flush previous file if it had positions.
            // If previous had no positions, it might be part of a
            // space-containing path — concatenate.
            match current_path.take() {
                Some(prev) if current_sels.is_empty() => {
                    // Concatenation heuristic: prev had no positions,
                    // so join with space.
                    current_path = Some(format!("{prev} {token}"));
                }
                Some(prev) => {
                    entries.push(FileEntry {
                        path: prev.into(),
                        selections: std::mem::take(&mut current_sels),
                    });
                    current_path = Some(token);
                }
                None => {
                    current_path = Some(token);
                }
            }
        }
    }

    // Flush last file
    if let Some(path) = current_path
        && !current_sels.is_empty()
    {
        entries.push(FileEntry {
            path: path.into(),
            selections: current_sels,
        });
    }

    Ok(entries)
}

/// Render file entries with inline content markers or ANSI colors.
///
/// Detects TTY/`NO_COLOR` automatically. Hierarchical output:
/// ```text
/// path
///   selection[, selection...]
///     content with markers/highlights
/// ```
///
/// Overlapping context ranges are merged into a single group with
/// comma-separated position headers.
pub fn render(entries: &[FileEntry], cwd: Option<&Path>, extent: Extent) -> anyhow::Result<String> {
    render_with_mode(entries, cwd, Mode::detect(), extent)
}

fn render_with_mode(
    entries: &[FileEntry],
    cwd: Option<&Path>,
    mode: Mode,
    extent: Extent,
) -> anyhow::Result<String> {
    let mut out = String::new();

    for (file_idx, entry) in entries.iter().enumerate() {
        if file_idx > 0 {
            out.push('\n');
        }

        // Resolve path
        let abs_path = if entry.path.is_absolute() {
            entry.path.clone()
        } else {
            let base = match cwd {
                Some(c) => c.to_path_buf(),
                None => std::env::current_dir()?,
            };
            base.join(&entry.path)
        };

        let display_path = relativize(&abs_path, cwd);

        // Read file
        let content = match std::fs::read_to_string(&abs_path) {
            Ok(c) => c,
            Err(e) => {
                out.push_str(&format!("{display_path}: {e}\n"));
                continue;
            }
        };
        let lines: Vec<&str> = content.lines().collect();

        // File path header
        if mode == Mode::Color {
            out.push_str(&format!("{}{display_path}{}\n", ansi::BOLD, ansi::RESET));
        } else {
            out.push_str(&format!("{display_path}\n"));
        }

        let groups = compute_groups(&entry.selections, lines.len(), extent);
        let show_headers = extent != Extent::Full;

        for group in &groups {
            if show_headers {
                let header: String = group
                    .sels
                    .iter()
                    .map(|s| display_sel(s))
                    .collect::<Vec<_>>()
                    .join(", ");
                if mode == Mode::Color {
                    out.push_str(&format!("  {}{header}{}\n", ansi::DIM, ansi::RESET));
                } else {
                    out.push_str(&format!("  {header}\n"));
                }
            }

            render_line_range(
                &mut out,
                &lines,
                group.first_line,
                group.last_line,
                &group.sels,
                mode,
            );
        }
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Grouping: merge selections whose visible line ranges overlap
// ---------------------------------------------------------------------------

/// A group of selections with overlapping visible line ranges.
struct Group<'a> {
    sels: Vec<&'a Selection>,
    first_line: usize,
    last_line: usize,
}

/// Whether a selection should be displayed as a cursor (just a position).
/// Includes true zero-width cursors and single-character selections that
/// editors sometimes report for cursors.
fn is_cursor_like(sel: &Selection) -> bool {
    sel.start == sel.end || (sel.start.line == sel.end.line && sel.end.col == sel.start.col + 1)
}

/// Format a selection for the group header line.
fn display_sel(sel: &Selection) -> String {
    if is_cursor_like(sel) {
        sel.start.to_string()
    } else {
        sel.to_string()
    }
}

/// Line range spanned by a selection (1-based).
fn sel_line_range(sel: &Selection) -> (usize, usize) {
    if is_cursor_like(sel) {
        (sel.start.line, sel.start.line)
    } else {
        (sel.start.line, sel.end.line)
    }
}

/// Group selections whose visible line ranges (including context) overlap.
fn compute_groups<'a>(sels: &'a [Selection], total_lines: usize, extent: Extent) -> Vec<Group<'a>> {
    if sels.is_empty() {
        return Vec::new();
    }

    let (ctx_b, ctx_a) = match extent {
        Extent::Exact => (0, 0),
        Extent::Lines { before, after } => (before, after),
        Extent::Full => (total_lines, total_lines),
    };

    let mut items: Vec<(&'a Selection, usize, usize)> = sels
        .iter()
        .map(|sel| {
            let (first, last) = sel_line_range(sel);
            let vis_first = first.saturating_sub(ctx_b).max(1);
            let vis_last = (last + ctx_a).min(total_lines);
            (sel, vis_first, vis_last)
        })
        .collect();
    items.sort_by_key(|&(_, f, _)| f);

    let mut groups: Vec<Group<'a>> = Vec::new();
    for (sel, first, last) in items {
        if let Some(g) = groups.last_mut()
            && first <= g.last_line + 1
        {
            g.sels.push(sel);
            g.last_line = g.last_line.max(last);
            continue;
        }
        groups.push(Group {
            sels: vec![sel],
            first_line: first,
            last_line: last,
        });
    }

    groups
}

// ---------------------------------------------------------------------------
// Unified line renderer: event-based annotation
// ---------------------------------------------------------------------------

/// Column-level events, ordered so SelEnd < Cursor < SelStart at equal column.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum EventKind {
    SelEnd,
    Cursor,
    SelStart,
}

/// Build column events for a specific line from the given selections.
fn line_events(sels: &[&Selection], line_num: usize) -> (Vec<(usize, EventKind)>, bool) {
    let mut events = Vec::new();
    let mut in_sel_at_start = false;

    for sel in sels {
        if is_cursor_like(sel) {
            if sel.start.line == line_num {
                events.push((sel.start.col, EventKind::Cursor));
            }
        } else if sel.start.line == sel.end.line {
            if sel.start.line == line_num {
                events.push((sel.start.col, EventKind::SelStart));
                events.push((sel.end.col, EventKind::SelEnd));
            }
        } else {
            // Multi-line selection
            if line_num == sel.start.line {
                events.push((sel.start.col, EventKind::SelStart));
            } else if line_num == sel.end.line {
                in_sel_at_start = true;
                events.push((sel.end.col, EventKind::SelEnd));
            } else if line_num > sel.start.line && line_num < sel.end.line {
                in_sel_at_start = true;
            }
        }
    }

    events.sort();
    (events, in_sel_at_start)
}

/// Render a range of lines with selection markers applied.
fn render_line_range(
    out: &mut String,
    lines: &[&str],
    first: usize,
    last: usize,
    sels: &[&Selection],
    mode: Mode,
) {
    for line_num in first..=last {
        if line_num == 0 || line_num > lines.len() {
            continue;
        }
        let line = lines[line_num - 1];
        let (events, in_sel) = line_events(sels, line_num);

        if events.is_empty() && !in_sel {
            emit_context_line(out, line, mode);
        } else {
            emit_annotated_line(out, line, &events, in_sel, mode);
        }
    }
}

/// Emit a plain context line (dimmed in color mode).
fn emit_context_line(out: &mut String, line: &str, mode: Mode) {
    out.push_str("    ");
    if mode == Mode::Color {
        out.push_str(ansi::DIM);
        out.push_str(line);
        out.push_str(ansi::RESET);
    } else {
        out.push_str(line);
    }
    out.push('\n');
}

/// Emit a line with column-level selection/cursor annotations.
///
/// The 4-space indent is never highlighted. ANSI state is fully managed
/// per-line (reset at end) so highlights never bleed into indents.
fn emit_annotated_line(
    out: &mut String,
    line: &str,
    events: &[(usize, EventKind)],
    in_sel_at_start: bool,
    mode: Mode,
) {
    out.push_str("    "); // indent — never highlighted

    let mut pos = 0usize;
    let mut in_sel = in_sel_at_start;

    if in_sel && mode == Mode::Color {
        out.push_str(ansi::INVERSE);
    }

    for &(col, kind) in events {
        let byte_pos = (col - 1).min(line.len());

        if byte_pos > pos {
            out.push_str(&line[pos..byte_pos]);
            pos = byte_pos;
        }

        match kind {
            EventKind::SelEnd => {
                match mode {
                    Mode::Markers => out.push(SEL_CLOSE),
                    Mode::Color => out.push_str(ansi::RESET),
                }
                in_sel = false;
            }
            EventKind::Cursor => match mode {
                Mode::Markers => out.push(CURSOR),
                Mode::Color => {
                    if pos < line.len() {
                        let end = next_char_end(line, pos);
                        if !in_sel {
                            out.push_str(ansi::INVERSE);
                        }
                        out.push_str(&line[pos..end]);
                        if !in_sel {
                            out.push_str(ansi::RESET);
                        }
                        pos = end;
                    } else {
                        out.push_str(ansi::INVERSE);
                        out.push(' ');
                        out.push_str(ansi::RESET);
                    }
                }
            },
            EventKind::SelStart => {
                match mode {
                    Mode::Markers => out.push(SEL_OPEN),
                    Mode::Color => out.push_str(ansi::INVERSE),
                }
                in_sel = true;
            }
        }
    }

    // Remaining text after last event
    if pos < line.len() {
        out.push_str(&line[pos..]);
    } else if line.is_empty() && in_sel {
        // Empty line inside a selection: show a small highlight so the
        // region appears visually connected.
        out.push(' ');
    }

    if in_sel && mode == Mode::Color {
        out.push_str(ansi::RESET);
    }

    out.push('\n');
}

/// Byte offset just past the next character.
fn next_char_end(s: &str, pos: usize) -> usize {
    let mut iter = s[pos..].char_indices();
    iter.next();
    iter.next().map(|(i, _)| pos + i).unwrap_or(s.len())
}

/// Check if a token looks like a position: `\d+:\d+` or `\d+:\d+-\d+:\d+`.
fn is_position(s: &str) -> bool {
    if let Some((left, right)) = s.split_once('-') {
        is_line_col(left) && is_line_col(right)
    } else {
        is_line_col(s)
    }
}

fn is_line_col(s: &str) -> bool {
    let Some((line, col)) = s.split_once(':') else {
        return false;
    };
    !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
        && !col.is_empty()
        && col.bytes().all(|b| b.is_ascii_digit())
}

/// Tokenize input respecting double-quoted strings.
fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
            continue;
        }
        if c == '"' {
            // Quoted token
            chars.next(); // consume opening quote
            let mut token = String::new();
            while let Some(&ch) = chars.peek() {
                if ch == '"' {
                    chars.next(); // consume closing quote
                    break;
                }
                token.push(ch);
                chars.next();
            }
            tokens.push(token);
        } else {
            // Unquoted token — until whitespace
            let mut token = String::new();
            while let Some(&ch) = chars.peek() {
                if ch.is_whitespace() {
                    break;
                }
                token.push(ch);
                chars.next();
            }
            tokens.push(token);
        }
    }

    tokens
}

#[cfg(test)]
mod tests {
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
        let result =
            render_with_mode(&entries, Some(dir.path()), Mode::Color, Extent::Exact).unwrap();
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
        let result =
            render_with_mode(&entries, Some(dir.path()), Mode::Color, Extent::Exact).unwrap();
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
    fn parse_compact_skips_terminal() {
        let entries =
            parse_compact("src/foo.rs 5:12\n/home/user/project $\nsrc/bar.rs 10:1").unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path.to_str().unwrap(), "src/foo.rs");
        assert_eq!(entries[1].path.to_str().unwrap(), "src/bar.rs");
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
}
