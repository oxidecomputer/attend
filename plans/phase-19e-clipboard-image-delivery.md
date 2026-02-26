# Phase 19E: Clipboard Image Delivery

**Dependencies**: Phase 19 (clipboard capture).
**Effort**: Small | **Risk**: Low

---

## Context

Clipboard capture (Phase 19) stages clipboard images as PNG files and
references them by absolute path in narration output. Two problems remain:

1. **Premature cleanup**: Images were deleted before the agent could read them
   (fixed in da62fbc by deferring to retention, but the lifecycle is still
   loose).
2. **Stale directory structure**: Staging is flat (`clipboard-staging/`) instead
   of session-scoped, and the `Read` permission is overly broad.

The original plan (attend clip + hook blocking + base64 stdout) was abandoned
after testing showed Claude Code's base64 image detection only works for tiny
images (<1KB). Real clipboard screenshots are orders of magnitude larger.

The simpler design: render clipboard images as standard markdown image tags
with full absolute staging paths. The filenames include nanosecond timestamps
and UUIDs, making them unguessable across sessions. The agent reads them
directly with the `Read` tool (which handles images natively). The protocol
strongly instructs the agent to read every clipboard image.

---

## Design

### Flow (agent session)

1. Daemon stages clipboard images to `clipboard-staging/<session>/<ts>-<uuid>.png`
2. Narration renders image events as `![🖼️ <id>](<staging-path>)`
3. Agent receives narration via `attend listen` → sees image tags
4. Agent uses `Read` tool on each image path (pre-authorized, no prompt)
5. Images persist in staging until retention cleanup

The `<id>` is the first 10 characters of the UUID (e.g., `aebb33d9-9`),
giving the agent a short label to reference the image. The full path is
in the markdown URL.

### Flow (yank / no session)

When yanking to clipboard (no agent to `Read` files):

1. Daemon stages images to `clipboard-staging/_local/`
2. On stop/yank, `copy_yanked_to_clipboard` encounters `ClipboardContent::Image`
3. It reads the PNG, base64-encodes it, and embeds inline:
   `![🖼️ <id>](data:image/png;base64,...)`
4. The staging file is moved to `archive/_local/`
5. The yanked markdown is self-contained — images travel with the text

Two modes:
- **With agent**: staging path in markdown → agent `Read`s directly
- **Without agent (yank)**: base64 inline in markdown → self-contained

### Security model

Cross-session image isolation relies on **filename unguessability**:
- Filenames include nanosecond timestamps + full UUID v4
  (e.g., `2026-02-26T19-56-47.178389000Z-aebb33d9-96c3-4b45-bb48-54b6d439ca22.png`)
- Session B's agent never sees session A's filenames (they only appear in
  session A's narration, delivered through session A's hooks)
- The `Read` permission is `Read(<cache>/attend/clipboard-staging/*)` —
  broad but access requires knowing the exact path

This is the same security model as any file on disk: if you know the path
you can read it, but the path is unguessable. No hook enforcement needed.

### Render changes

**Agent delivery** (narration markdown sent via hooks):

`ClipboardContent::Image { path }` renders as a markdown image tag:

```markdown
![🖼️ aebb33d9-9](/Users/oxide/.cache/attend/clipboard-staging/s0/2026-02-26T19-56-47.178389000Z-aebb33d9-96c3-4b45-bb48-54b6d439ca22.png)
```

The `<id>` (first 10 chars of UUID) is derived from the path at render time.
The `path` field in `ClipboardContent::Image` stays as the full staging path.

**Yank delivery** (markdown copied to system clipboard):

```markdown
![🖼️ aebb33d9-9](data:image/png;base64,iVBORw0KGgo...)
```

The image is read from staging, base64-encoded, and embedded inline.
The staging file is then moved to archive.

### Narration protocol update

Add clipboard images to the event type list in the protocol:

> **Clipboard images** — content the user copied to the clipboard as an image
> (e.g., a screenshot). Rendered as a markdown image tag with an absolute path:
>
> ![🖼️ aebb33d9-9](/path/to/clipboard-staging/session/timestamp-uuid.png)
>
> **You must `Read` every clipboard image path when you encounter one.** These
> are ephemeral — the user copied this image while narrating, and it will not
> be available indefinitely. Treat clipboard images with the same priority as
> spoken words: they are part of what the user is communicating to you.

### Cleanup changes

- `clipboard_staging_dir()` becomes session-scoped: takes `Option<&SessionId>`
  (same pattern as `browser_staging_dir`, `shell_staging_dir`)
- `clean.rs`: add `clipboard-staging/` cleanup alongside `archive/`,
  same retention cutoff. Reuse `clean_archive_dir` (walk session subdirs,
  prune by mtime).
- The existing `Read` permission on `clipboard-staging/*` stays as-is
  (already installed by Phase 19)

---

## Task breakdown (red-green TDD)

### Phase E1: Types + stubs (compiles)

| # | Task | Files |
|---|------|-------|
| 1 | Make `clipboard_staging_dir` session-scoped: `fn clipboard_staging_dir(Option<&SessionId>)` | `narrate.rs` |
| 2 | Update daemon to pass session ID to clipboard staging | `narrate/clipboard_capture.rs`, `narrate/capture.rs`, `narrate/record.rs` |
| 3 | Add render mode to `render_markdown` (`enum RenderMode { Agent, Yank }`) for image handling | `narrate/render.rs` |
| 4 | Stub yank image embedding in `copy_yanked_to_clipboard` | `narrate/record.rs` |

### Phase E2: Tests (red)

| # | Task | Files |
|---|------|-------|
| 5 | Render test: `clipboard_image_renders_as_markdown_image_tag_with_id` (update existing `clipboard_image_renders_as_image_tag` from Phase 19C) | `narrate/merge/tests/render.rs` |
| 6 | Render test: `clipboard_image_yank_renders_as_base64` | `narrate/merge/tests/render.rs` |
| 7 | Yank tests: `yank_archives_clipboard_images`, `yank_embeds_base64_in_markdown` | `narrate/tests.rs` |
| 8 | Clean test: `clean_prunes_session_scoped_clipboard_staging` | `narrate/clean.rs` or `narrate/tests.rs` |

### Phase E3: Implementation (green)

| # | Task | Files |
|---|------|-------|
| 9 | Update render: agent mode outputs `![🖼️ <id>](path)`, yank mode reads file + base64 embeds | `narrate/render.rs` |
| 10 | Implement yank image handling: read PNG → base64 → inline, move staging → archive | `narrate/record.rs` |
| 11 | Update `clean.rs`: prune `clipboard-staging/<session>/` dirs by retention | `narrate/clean.rs` |

### Phase E4: Docs

| # | Task | Files |
|---|------|-------|
| 12 | Update narration protocol: clipboard image description + strong read instruction | `agent/messages/narration_protocol.md` |
| 13 | Update setup guide if needed | `docs/setup.md` |

---

## Key files to modify

- `src/narrate.rs` — session-scoped `clipboard_staging_dir`
- `src/narrate/clipboard_capture.rs` — accept session-scoped staging dir
- `src/narrate/capture.rs` — pass session ID to clipboard thread
- `src/narrate/record.rs` — pass session ID, yank image base64 embedding
- `src/narrate/render.rs` — `RenderMode`, image tag with ID (agent), base64 (yank)
- `src/narrate/clean.rs` — session-scoped staging cleanup
- `src/agent/messages/narration_protocol.md` — clipboard image instructions

## Existing code to reuse

- `crate::narrate::dir_key(Option<&SessionId>)` — session → dir key
- `crate::narrate::clean::clean_archive_dir` — walk session subdirs, prune by mtime
- `crate::narrate::render::render_markdown` — extend with RenderMode
- Phase 19's `stage_image_png` — already writes to staging dir, just needs session scoping
- `base64` crate (via `arboard`) or `data_encoding` — for yank base64 encoding

## Implementation notes

**Render mode**: `render_markdown` gains a `RenderMode` parameter:
- `Agent`: `![🖼️ <id>](staging-path)` — agent reads the file
- `Yank`: `![🖼️ <id>](data:image/png;base64,...)` — self-contained

The yank path reads the PNG file at render time. If the file doesn't exist
(race condition), the image is rendered as `[🖼️ <id>: image unavailable]`.

**`capture::start` signature**: Already takes `clipboard_capture: bool`.
Now also needs `session_id: Option<&SessionId>` (or the staging dir path)
so the clipboard thread writes to the session-scoped dir.

**Phase 19 cleanup to revisit**: `record.rs` has a comment block (from da62fbc)
about not cleaning up clipboard staging eagerly. This is still correct — images
must persist until retention cleanup (agent may not read them immediately).
The `clean_flat_dir` call in `clean.rs` needs to change to `clean_archive_dir`
on the session-scoped `clipboard-staging/` directory.

**Existing `Read` permission**: Phase 19 already installed
`Read(<cache>/attend/clipboard-staging/*)`. This stays as-is — the glob
covers all session subdirectories.

---

## Verification

1. `cargo fmt --check` + `cargo clippy` + `cargo test` — all clean
2. Manual (with session): record → copy image → stop → `attend listen` →
   see `![🖼️ <id>](path)` in narration → `Read` the path → see image
3. Manual (yank, no session): record → copy image → yank → verify clipboard
   markdown has `![🖼️ <id>](data:image/png;base64,...)`
4. Manual: verify `attend narrate clean` removes old staging + archive images
5. Verify all existing 473 tests pass alongside new tests
