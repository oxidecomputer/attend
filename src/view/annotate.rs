use crate::state::Selection;

use super::{ansi, Extent, Mode, CURSOR, SEL_CLOSE, SEL_OPEN};

/// A group of selections with overlapping visible line ranges.
pub(super) struct Group<'a> {
    pub sels: Vec<&'a Selection>,
    pub first_line: usize,
    pub last_line: usize,
}

/// Whether a selection should be displayed as a cursor (just a position).
/// Includes true zero-width cursors and single-character selections that
/// editors sometimes report for cursors.
fn is_cursor_like(sel: &Selection) -> bool {
    sel.start == sel.end || (sel.start.line == sel.end.line && sel.end.col == sel.start.col + 1)
}

/// Format a selection for the group header line.
pub(super) fn display_sel(sel: &Selection) -> String {
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
pub(super) fn compute_groups<'a>(
    sels: &'a [Selection],
    total_lines: usize,
    extent: Extent,
) -> Vec<Group<'a>> {
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
pub(super) fn render_line_range(
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
