use crate::state::{Col, Line, Selection};

use super::{CURSOR, Extent, Mode, SEL_CLOSE, SEL_OPEN, ansi};

/// A group of selections with overlapping visible line ranges.
pub(super) struct Group<'a> {
    /// Selections belonging to this group.
    pub sels: Vec<&'a Selection>,
    /// First visible line (1-based).
    pub first_line: Line,
    /// Last visible line (1-based).
    pub last_line: Line,
}

impl<'a> Group<'a> {
    /// Group selections whose visible line ranges (including context) overlap.
    pub fn compute(sels: &'a [Selection], total_lines: usize, extent: Extent) -> Vec<Self> {
        if sels.is_empty() || total_lines == 0 {
            return Vec::new();
        }

        let total = Line::new(total_lines).unwrap();

        let (ctx_b, ctx_a) = match extent {
            Extent::Exact => (0, 0),
            Extent::Lines { before, after } => (before, after),
            Extent::Full => (total_lines, total_lines),
        };

        let mut items: Vec<(&'a Selection, Line, Line)> = sels
            .iter()
            .map(|sel| {
                let (vis_first, vis_last) = if extent == Extent::Full {
                    (Line::new(1).unwrap(), total)
                } else {
                    let (first, last) = sel.line_range();
                    let vf = first.saturating_sub(ctx_b).min(total);
                    let vl = last.saturating_add(ctx_a).min(total).max(vf);
                    (vf, vl)
                };
                (sel, vis_first, vis_last)
            })
            .collect();
        items.sort_by_key(|&(_, f, _)| f);

        let mut groups: Vec<Self> = Vec::new();
        for (sel, first, last) in items {
            if let Some(g) = groups.last_mut()
                && first <= g.last_line.saturating_add(1)
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
}

// ---------------------------------------------------------------------------
// Unified line renderer: event-based annotation
// ---------------------------------------------------------------------------

/// Column-level events, ordered so SelEnd < Cursor < SelStart at equal column.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum EventKind {
    /// End of a selection range.
    SelEnd,
    /// A cursor position.
    Cursor,
    /// Start of a selection range.
    SelStart,
}

/// Build column events for a specific line from the given selections.
pub(super) fn line_events(sels: &[&Selection], line_num: Line) -> (Vec<(Col, EventKind)>, bool) {
    let mut events = Vec::new();
    let mut in_sel_at_start = false;

    for sel in sels {
        if sel.is_cursor_like() {
            if sel.start.line == line_num {
                events.push((sel.start.col, EventKind::Cursor));
            }
        } else if sel.start.line == sel.end.line {
            if sel.start.line == line_num {
                // Normalize column order for reversed single-line selections.
                let (sc, ec) = if sel.start.col <= sel.end.col {
                    (sel.start.col, sel.end.col)
                } else {
                    (sel.end.col, sel.start.col)
                };
                events.push((sc, EventKind::SelStart));
                events.push((ec, EventKind::SelEnd));
            }
        } else {
            // Multi-line selection — normalize line order.
            let (start, end) = if sel.start.line <= sel.end.line {
                (&sel.start, &sel.end)
            } else {
                (&sel.end, &sel.start)
            };
            if line_num == start.line {
                events.push((start.col, EventKind::SelStart));
            } else if line_num == end.line {
                in_sel_at_start = true;
                events.push((end.col, EventKind::SelEnd));
            } else if line_num > start.line && line_num < end.line {
                in_sel_at_start = true;
            }
        }
    }

    events.sort();
    (events, in_sel_at_start)
}

/// Render a range of lines with selection markers applied.
///
/// Dedents the snippet by stripping the common leading whitespace shared
/// by all non-empty lines, so deeply nested code is left-aligned.
pub(super) fn render_line_range(
    out: &mut String,
    lines: &[&str],
    first: Line,
    last: Line,
    sels: &[&Selection],
    mode: Mode,
) {
    let out_start = out.len();

    // Common leading whitespace across non-empty lines in this range.
    let trim = (first.get()..=last.get())
        .map(|i| lines[i - 1])
        .filter(|l| !l.is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);

    for line_num in first.get()..=last.get() {
        let raw = lines[line_num - 1];
        let line = &raw[trim.min(raw.len())..];
        let ln = Line::new(line_num).unwrap();
        let (events, in_sel) = line_events(sels, ln);

        if events.is_empty() && !in_sel {
            emit_context_line(out, line, mode);
        } else {
            // Shift column events left by the trim amount.
            let adjusted: Vec<(Col, EventKind)> = events
                .into_iter()
                .map(|(col, kind)| {
                    let c = col.get().saturating_sub(trim).max(1);
                    (Col::new(c).unwrap(), kind)
                })
                .collect();
            emit_annotated_line(out, line, &adjusted, in_sel, mode);
        }
    }

    // Ensure brackets are balanced in Markers mode. For each non-cursor
    // selection, if one endpoint is rendered but the other is not, insert
    // the missing bracket so ⟦/⟧ always pair up.
    if mode == Mode::Markers && out.len() > out_start {
        let mut prepend_count = 0usize;
        let mut append_count = 0usize;
        for sel in sels {
            if sel.is_cursor_like() {
                continue;
            }
            let (sl, el) = sel.line_range();
            let start_in = sl >= first && sl <= last;
            let end_in = el >= first && el <= last;
            if start_in && !end_in {
                append_count += 1;
            } else if !start_in && end_in {
                prepend_count += 1;
            }
        }
        // Append missing close brackets before trailing newline.
        for _ in 0..append_count {
            if out.ends_with('\n') {
                out.pop();
                out.push(SEL_CLOSE);
                out.push('\n');
            }
        }
        // Prepend missing open brackets after the first indent.
        if prepend_count > 0 {
            let insert_pos = out_start + "    ".len();
            let char_len = SEL_OPEN.len_utf8();
            for i in 0..prepend_count {
                out.insert(insert_pos + i * char_len, SEL_OPEN);
            }
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
    events: &[(Col, EventKind)],
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
        let byte_pos = (col.get() - 1).min(line.len());

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
