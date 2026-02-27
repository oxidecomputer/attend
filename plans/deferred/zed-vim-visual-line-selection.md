# Zed: Vim Visual-Line Selections Not Visible via SQLite

**Date**: 2026-02-23
**Status**: Upstream limitation (Zed), no attend-side fix possible
**Impact**: `V` (visual line) selections in Zed's vim mode are invisible to attend;
regular `v` selections with motion and mouse selections work fine.

## Summary

Zed's vim visual-line mode (`V`) does not persist the expanded selection range
to the `editor_selections` SQLite table. The table only ever contains the raw
cursor position (a 1-byte block-cursor range). The full-line expansion is a
display-time computation driven by a transient `line_mode` flag on the
selections collection, which is never written to the database.

This means attend (and any other consumer of `editor_selections`) cannot
distinguish a visual-line selection from a normal-mode cursor.

## What works and what doesn't

| Scenario | Persisted to DB | Visible to attend |
|----------|----------------|-------------------|
| Normal mode cursor | `(N, N+1)` — 1-byte block cursor | Yes (as cursor) |
| `v` + motion (visual char) | `(start, end)` — actual range | Yes (as selection) |
| `V` (visual line) | `(N, N+1)` — just the cursor | No (looks like cursor) |
| Mouse selection in any mode | `(start, end)` — actual range | Yes (as selection) |

## Root cause in Zed

### 1. Vim sets `line_mode` as a display flag, not a data mutation

When entering visual-line mode, vim calls `sync_vim_settings` which propagates
the mode to the editor:

[`crates/vim/src/vim.rs:2063`](https://github.com/zed-industries/zed/blob/5ef898d61ebf71c8d9b10cb5c975551b5715ef67/crates/vim/src/vim.rs#L2063):
```rust
line_mode: matches!(self.mode, Mode::VisualLine),
```

[`crates/vim/src/vim.rs:2080`](https://github.com/zed-industries/zed/blob/5ef898d61ebf71c8d9b10cb5c975551b5715ef67/crates/vim/src/vim.rs#L2080):
```rust
editor.selections.set_line_mode(state.line_mode);
```

This sets a boolean flag on the `SelectionsCollection`:

[`crates/editor/src/selections_collection.rs:601`](https://github.com/zed-industries/zed/blob/5ef898d61ebf71c8d9b10cb5c975551b5715ef67/crates/editor/src/selections_collection.rs#L601):
```rust
pub fn set_line_mode(&mut self, line_mode: bool) {
    self.line_mode = line_mode;
}
```

### 2. `line_mode` only expands at display time

The `all_adjusted` method expands selections to full lines — but only when
called for rendering, not for persistence:

[`crates/editor/src/selections_collection.rs:165-174`](https://github.com/zed-industries/zed/blob/5ef898d61ebf71c8d9b10cb5c975551b5715ef67/crates/editor/src/selections_collection.rs#L165-L174):
```rust
pub fn all_adjusted(&self, snapshot: &DisplaySnapshot) -> Vec<Selection<Point>> {
    let mut selections = self.all::<Point>(&snapshot);
    if self.line_mode {
        for selection in &mut selections {
            let new_range = snapshot.expand_to_line(selection.range());
            selection.start = new_range.start;
            selection.end = new_range.end;
        }
    }
    selections
}
```

### 3. DB persistence reads raw anchors, not adjusted selections

The persistence code in `selections_did_change` reads from the raw anchor
data, bypassing the `line_mode` expansion entirely:

[`crates/editor/src/editor.rs:3676-3684`](https://github.com/zed-industries/zed/blob/5ef898d61ebf71c8d9b10cb5c975551b5715ef67/crates/editor/src/editor.rs#L3676-L3684):
```rust
let db_selections = selections
    .iter()
    .map(|selection| {
        (
            selection.start.to_offset(&snapshot).0,
            selection.end.to_offset(&snapshot).0,
        )
    })
    .collect();
```

The result: the DB always stores the unexpanded cursor `(N, N+1)`, regardless
of `line_mode`.

### 4. `switch_mode` doesn't expand selection endpoints for visual-line either

When entering visual mode, `switch_mode` only extends the selection by one
character (for the block cursor), it doesn't expand to the full line:

[`crates/vim/src/vim.rs:1274-1278`](https://github.com/zed-industries/zed/blob/5ef898d61ebf71c8d9b10cb5c975551b5715ef67/crates/vim/src/vim.rs#L1274-L1278):
```rust
} else if !last_mode.is_visual() && mode.is_visual() {
    if selection.is_empty() {
        selection.end = movement::right(map, selection.start);
    }
}
```

This code is shared across all visual modes (`v`, `V`, `Ctrl-V`). The
line-mode expansion is deferred entirely to the `line_mode` flag.

## How attend reads selections

attend queries Zed's SQLite database for active editor selections:

[`src/editor/zed/db.rs:44-50`](https://github.com/oxidecomputer/attend/blob/755627eaf0f61a3c3285e4733ae5dd4c4a7bdc39/src/editor/zed/db.rs#L44-L50):
```rust
"SELECT e.path, es.start, es.end \
 FROM items i \
 JOIN editors e ON i.item_id = e.item_id AND i.workspace_id = e.workspace_id \
 LEFT JOIN editor_selections es \
   ON e.item_id = es.editor_id AND e.workspace_id = es.workspace_id \
 WHERE i.kind = 'Editor' AND i.active = 1 \
 ORDER BY e.path, es.start",
```

The `(start, end)` byte offsets are resolved to `(line:col, line:col)` and
classified as cursor-like or selection via `is_cursor_like`:

[`src/state/resolve.rs:119-122`](https://github.com/oxidecomputer/attend/blob/755627eaf0f61a3c3285e4733ae5dd4c4a7bdc39/src/state/resolve.rs#L119-L122):
```rust
pub fn is_cursor_like(&self) -> bool {
    self.start == self.end
        || (self.start.line == self.end.line && self.end.col.get() == self.start.col.get() + 1)
}
```

The second clause catches Zed's 1-byte block cursor representation (`start,
start+1`), treating it as a cursor. This is correct: in normal mode, the
block cursor IS a cursor. The problem is that vim visual-line mode produces
the same 1-byte range in the DB, so attend can't tell them apart.

## Why no attend-side fix is possible

The `line_mode` flag is transient runtime state in Zed's editor. It is not
written to the SQLite database, and there is no other table or column that
records the current vim mode. attend has no way to know whether a 1-byte
selection in the DB represents a normal-mode cursor or a visual-line selection.

## Potential Zed-side fixes

1. **Persist `line_mode` in `editor_selections`**: Add a `line_mode` boolean
   column. Consumers can expand selections accordingly.

2. **Persist expanded selections**: In `selections_did_change`, use
   `all_adjusted` instead of raw anchors when `line_mode` is true.

3. **Persist vim mode**: Add a column or separate table for the current vim
   mode, so consumers can apply the expansion themselves.

Option 2 is the simplest and most compatible with existing consumers.
