# Phase 16: Yank-to-Clipboard

**Dependencies**: Phase 13 (no-session support).
**Effort**: Small-Medium | **Risk**: Low

---

## Motivation

Today, `stop` transcribes and stages content for automatic delivery via the
`attend listen` hook. The user has no opportunity to review, edit, or redirect
the output. Yank is an alternate stop that renders the narration to markdown
and places it on the system clipboard instead of staging it for hook delivery.
The user can then:

- Paste directly into the agent (if they don't want to edit).
- Paste into their editor, revise, then paste into the agent.
- Review and discard by simply not pasting.

This also enables **narration without an agent**: the user can record, capture
editor context, and get clipboard output without any Claude Code session. This
is useful for drafting messages, documenting what you're doing, or composing
context for a different tool. Phase 13 (no-session support) ensures full
capture functionality in this case.

---

## Design

### Yank sentinel

`~/.cache/attend/yank` (alongside `stop` and `flush`).

The daemon needs to know about yank because it must write output to a different
directory than `pending/`, which the hook delivery path monitors. If the daemon
wrote to `pending/` and the CLI cleaned up after, there would be a race with
`attend hook pre-tool-use` delivering the same content.

### Daemon behavior

`check_yank()` is identical to `check_stop()` except `transcribe_and_write`
writes to `yanked/<id>/` instead of `pending/<id>/`. The hook path never looks
at `yanked/`, so there is no race.

When no session exists, the `_local` fallback from phase 13 applies:
`yanked/_local/`.

### CLI: `attend narrate yank`

1. Write the `yank` sentinel (not `stop`).
2. Wait for daemon to exit (poll `record.lock` removal, same as `stop()`).
3. Daemon has written output to `yanked/<id>/`.
4. Read and render: same logic as `read_pending()` — deserialize events,
   filter to cwd, relativize paths, render to markdown.
5. If no content: print "No narration content." and exit **without touching
   the clipboard** (preserve whatever the user already has there).
6. Copy the rendered markdown to the system clipboard.
7. Clean up the `yanked/<id>/` directory.
8. Print a confirmation with a character/line count so the user knows it
   worked.

### Clipboard access

Use the `arboard` crate for cross-platform clipboard access. It supports
macOS (AppKit), Linux (X11/Wayland via `wl-copy`/`xclip`), and Windows.
Zero platform-specific code in attend.

---

## Keybinding

| Action | macOS | Linux |
|--------|-------|-------|
| **Yank to clipboard** | `cmd-}` | `super-}` |

New Zed task: `"attend: yank narration"` → `attend narrate yank`

Uses `"hide": "always"` and `"reveal": "never"` (same as toggle/start).

Yank uses the existing stop chime (the daemon stops normally — only the output
destination differs). If the yank produces no content, the empty chime from
phase 14 plays instead.

---

## Task breakdown

| # | Task | Depends On | Files |
|---|------|------------|-------|
| 1 | `yanked_dir()` + yank sentinel path | — | `narrate.rs` |
| 2 | `check_yank()` in daemon (write to `yanked/` dir) | 1 | `narrate/record.rs` |
| 3 | `attend narrate yank` CLI subcommand | 2 | `cli/narrate.rs` |
| 4 | Clipboard write via `arboard` | 3 | `cli/narrate.rs`, `Cargo.toml` |
| 5 | Zed task + keybinding for yank | — | `editor/zed.rs`, `editor/zed/keybindings.rs` |
| 6 | Tests: yank output, empty yank preserves clipboard, `yanked/` cleanup | All | tests |

---

## Verification

- Start recording, speak, press yank hotkey → hear stop chime, narration
  appears on clipboard. Paste into editor to verify.
- Start recording, say nothing, press yank hotkey → hear empty chime,
  clipboard unchanged.
- Start recording without an agent session, speak, yank → clipboard has
  full narration including browser/ext selections.
- After yank, `attend listen` hook finds nothing to deliver (no race).

---

## Open questions

1. **Yank without stopping**: Should `attend narrate yank` also support a
   `--continue` flag (flush + clipboard instead of stop + clipboard)? This
   would let the user grab intermediate narration without ending the session.
   Deferred to a future iteration.
