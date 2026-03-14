# Zed Diff View Investigation

## Problem

When the user opens a diff in Zed (`zed --diff old new`), attend cannot see
cursor positions, selections, or file content from the diff view. The user
might be reviewing a diff and narrating observations, but the agent receives
no visual context about what they're looking at.

## Investigation findings (2026-02-23)

### Two barriers, not one

**Barrier 1: SQLite database invisibility**

`zed --diff` creates `FileDiffView` (two-file) or `MultiDiffView` (directory)
items in the workspace. Neither implements Zed's `SerializableItem` trait, so
they never write to the `items`, `editors`, or `editor_selections` SQLite
tables. attend's Zed integration queries:

```sql
SELECT e.path, es.start, es.end
FROM items i
JOIN editors e ON i.item_id = e.item_id AND i.workspace_id = e.workspace_id
LEFT JOIN editor_selections es ON ...
WHERE i.kind = 'Editor' AND i.active = 1
```

This query returns nothing when the active item is a diff view. The
`ProjectDiff` item (Zed's "Uncommitted Changes" panel) IS serializable with
kind `"ProjectDiff"`, but it uses a separate `ProjectDiffDb` table that only
stores the diff base reference, not cursor/selection state.

Zed explicitly excluded selection persistence for multi-buffer editors in
[PR #25140](https://github.com/zed-industries/zed/pull/25140), and all diff
views use multi-buffer editors internally.

**Barrier 2: GPUI blocks accessibility API**

Zed renders all text using GPUI, its custom GPU-accelerated rendering
framework. GPUI does not use standard AppKit `NSTextView` controls, so the
macOS Accessibility API (`AXSelectedText`, `AXValue`, etc.) cannot read
content from Zed's editor panes. This was confirmed empirically earlier in
attend's development.

This means the Phase 12a Accessibility capture (which works for Safari,
iTerm2, Chrome, etc.) cannot read from Zed diff views either.

### What IS available

- **Window title**: Zed's window title is accessible via the Accessibility API
  and typically includes the workspace name. This tells us the user is in Zed,
  but not what they're looking at.

- **Active item kind**: If we queried `items WHERE active = 1` without the
  `kind = 'Editor'` filter, we could detect that a `ProjectDiff` item is
  active. We wouldn't get selections, but we'd know "user is reviewing
  uncommitted changes."

## Possible solutions (not currently planned)

### Upstream Zed changes

1. **Extend `SerializableItem` to diff views**: If Zed serialized
   `FileDiffView` and `MultiDiffView` to the database with cursor/selection
   state, attend could read them. Unlikely in the short term — Zed explicitly
   chose not to persist multi-buffer selections.

2. **GPUI accessibility improvements**: If Zed implements the `NSAccessibility`
   protocol on GPUI text views, the AX API could read selected text. Zed has
   been adding accessibility features, but comprehensive text selection
   reporting is a significant effort.

3. **Zed extension API (ACP)**: A future Zed extension/agent communication
   protocol could expose diff view state programmatically. This is the most
   likely long-term solution but doesn't exist yet.

### Workarounds in attend

1. **Detect diff view active state**: Query for `ProjectDiff` items in the
   SQLite database to detect when the user has the uncommitted changes panel
   open. Emit a lightweight event like "user is reviewing uncommitted changes
   in Zed" without file/selection details.

2. **Git diff as proxy**: When we detect the user is in a diff view (via the
   database or window title), run `git diff` and include the output as context.
   This provides the diff content but not what the user is specifically looking
   at within it.

3. **Verbal narration**: The user describes what they see in the diff. This
   already works today and is the current workaround.

## Status

Open. No implementation planned. Revisit when:
- Zed adds accessibility support for GPUI text views
- Zed adds an extension API that exposes editor state
- Zed extends `SerializableItem` to diff views
