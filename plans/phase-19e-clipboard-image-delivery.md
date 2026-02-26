# Phase 19E: Clipboard Image Polish

**Dependencies**: Phase 19 (clipboard capture).
**Effort**: Small | **Risk**: Low

---

## Context

Phase 19 clipboard image capture works end-to-end: images are staged as
PNGs, rendered as `![clipboard](path)` in narration, and the agent can
`Read` them directly. Three small improvements remain.

---

## Changes

### 1. Session-scoped staging

`clipboard_staging_dir()` currently returns a flat directory. Change it to
take `Option<&SessionId>` and return `clipboard-staging/<session>/` (same
pattern as `browser_staging_dir`, `shell_staging_dir`). This aligns with
all other staging directories and enables per-session retention cleanup.

**Files**: `narrate.rs`, `narrate/clipboard_capture.rs`, `narrate/capture.rs`,
`narrate/record.rs`

### 2. Yank base64 embedding

When yanking to clipboard (no agent), embed images inline as
`![clipboard](data:image/png;base64,...)` so the yanked markdown is
self-contained. Move the staging file to archive after encoding.

Requires a render mode parameter on `render_markdown`:
`enum RenderMode { Agent, Yank }`. Agent mode keeps `![clipboard](path)`.
Yank mode reads the PNG and base64-encodes it inline. If the file is
missing (race), render as `[clipboard image unavailable]`.

**Files**: `narrate/render.rs`, `narrate/record.rs`

### 3. Protocol: strong read instruction

Update the narration protocol to tell the agent clipboard images must be
read:

> **You must `Read` every clipboard image path when you encounter one.**
> These are ephemeral — the user copied this image while narrating, and it
> will not be available indefinitely.

**Files**: `agent/messages/narration_protocol.md`

---

## Task breakdown (red-green TDD)

| # | Task | Files |
|---|------|-------|
| 1 | Make `clipboard_staging_dir` session-scoped | `narrate.rs` |
| 2 | Pass session ID through capture to clipboard thread | `narrate/capture.rs`, `narrate/clipboard_capture.rs`, `narrate/record.rs` |
| 3 | Add `RenderMode` enum, thread through `render_markdown` | `narrate/render.rs` |
| 4 | Write tests: `clipboard_image_yank_renders_as_base64`, `yank_embeds_base64_in_markdown`, `clean_prunes_session_scoped_clipboard_staging` | render + narrate tests |
| 5 | Implement yank base64 embedding (read PNG, encode, inline) | `narrate/render.rs`, `narrate/record.rs` |
| 6 | Update `clean.rs`: `clean_archive_dir` on session-scoped `clipboard-staging/` (replace `clean_flat_dir`) | `narrate/clean.rs` |
| 7 | Update narration protocol | `agent/messages/narration_protocol.md` |

---

## Verification

1. `cargo fmt --check` + `cargo clippy` + `cargo test` — all clean
2. Manual (with session): record → copy image → stop → `attend listen` →
   see `![clipboard](path)` → `Read` path → see image
3. Manual (yank): record → copy image → yank → verify clipboard markdown
   has `![clipboard](data:image/png;base64,...)`
4. Manual: `attend narrate clean` removes old session-scoped staging dirs
5. All existing 473 tests pass alongside new tests
