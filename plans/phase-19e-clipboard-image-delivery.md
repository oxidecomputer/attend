# Phase 19E: Clipboard Image Delivery via `attend clip`

**Dependencies**: Phase 19 (clipboard capture).
**Effort**: Medium | **Risk**: Low-Medium (touches hook state machine)

---

## Context

Clipboard capture (Phase 19) stages clipboard images as PNG files and
references them by absolute path in narration output. This has two problems:

1. **Premature cleanup**: Images were deleted before the agent could read them
   (fixed in da62fbc by deferring to retention, but the lifecycle is still
   loose — there's no guarantee the agent reads them before they age out).
2. **Permission scoping**: The `Read` permission on `clipboard-staging/*` lets
   any agent session read any other session's clipboard images.

This change introduces `attend clip`, a session-scoped image delivery command
with hook-enforced blocking to guarantee the agent reads every clipboard
image.

---

## Design

### Flow (agent session)

1. Daemon stages clipboard images to `clipboard-staging/<session>/<ts>-<uuid>.png`
2. Narration renders image events as `🖼️ <id>` placeholders
3. On stop/flush, narration JSON is written with the staging filenames
4. Agent receives narration via `attend listen` → sees image placeholders
   → new listener starts in background (same `attend listen` round trip)
5. Hook detects pending files in `clipboard-staging/<session>/` → blocks
   all actions except `attend clip` and `attend listen`
6. Agent runs bare `attend clip` (no arguments)
7. PreToolUse hook:
   - Uses `updatedInput` to rewrite command to `attend clip --session <sid>`
   - Uses `additionalContext` to report which image ID is being delivered
8. `attend clip --session <sid>` serves oldest PNG from staging:
   reads file → outputs `data:image/png;base64,...` → archives file
9. If more images remain in staging, next hook check blocks again
10. Once staging dir empty, block lifts; normal operation resumes

The agent never sees paths or session IDs. The permission whitelist is
`Bash(attend clip)` (exact match, no wildcard) — only the bare command
with no arguments is pre-authorized. If the agent tries to pass `--session`
or any other arguments, the command doesn't match the whitelist and is
denied. The hook injects `--session` via `updatedInput` after authorization.

### Flow (yank / no session)

When yanking to clipboard (no agent to run `attend clip`):

1. Daemon stages images to `clipboard-staging/_local/`
2. On stop/yank, `copy_yanked_to_clipboard` encounters `ClipboardContent::Image`
3. It reads the PNG, base64-encodes it, and embeds it inline:
   `![clipboard](data:image/png;base64,...)`
4. The staging file is moved to `archive/_local/`
5. The yanked markdown is self-contained — images travel with the text

Two modes:
- **With agent**: staging → `attend clip` (base64 delivery) → archive
- **Without agent (yank)**: staging → base64 inline in markdown → archive

### `attend clip` subcommand

New CLI subcommand (hidden, internal like `browser-bridge`):

```
attend clip --session <session-id>
```

The `--session` flag is always injected by the PreToolUse hook. The agent
invokes bare `attend clip`; the hook rewrites it.

1. Reads `clipboard-staging/<session>/`, sorts entries by name (oldest first)
2. Takes the first `.png` file
3. Reads the file, base64-encodes it
4. Outputs `data:image/png;base64,...` to stdout (sole output — triggers
   Claude Code's image rendering)
5. Moves the file to `archive/<session>/<filename>` (create dir if needed)
6. Exits 0

If no files exist (race or stale state), outputs nothing and exits 0.

### PreToolUse hook for `attend clip`

Detection: parse `attend clip` from the bash command string (same pattern as
`detect_listen_command`).

**PreToolUse**:

1. If the calling session is not Active: **deny** (session moved/inactive).
2. If pending narration exists (`NarrationReady`): **deny**. The agent must
   run `attend listen` first.
3. If receiver is not alive (`StartReceiver`): **deny**. The agent must
   restart the listener first. The listener must always be running before
   the agent does anything else, including consuming clipboard images.
4. List `.png` files in `clipboard-staging/<session>/`, sorted.
5. If none exist: **allow** silently (no rewrite needed, command is a no-op).
6. Take the first filename. Extract image ID (first 6 chars of UUID).
   **Allow** with:
   - `updatedInput`: rewrite command to `attend clip --session <session-id>`
   - `additionalContext`: `"Delivering clipboard image <id>."`

**PostToolUse**: Silent.

### State machine changes

The hook state machine gains a new blocking condition. In priority order
within the Active relation for general (non-listen, non-clip) hooks:

1. **Pending narration** → `Block(NarrationReady)` (existing, unchanged)
2. **Receiver alive** → check clipboard, else fall through (existing check, extended)
   - If receiver alive and **clipboard images pending** → `Block(ClipboardReady)` (new)
   - If receiver alive and no clipboard → `Silent` (existing)
3. **Receiver not alive** → `StartReceiver` (existing, unchanged)
4. ... safety valve, rest unchanged

**What `ClipboardReady` blocks:**
- General hooks (Stop/PreToolUse/PostToolUse) for non-listen, non-clip tools
- End-turn (Stop hook)

**What `ClipboardReady` does NOT block:**
- `attend listen` — the listener must always be able to restart. This is
  the highest priority action.
- `attend clip` — this is how the agent clears the block.

**What blocks `attend clip`:**
- Missing listener (no receiver alive) → must restart listener first
- Pending narration → must `attend listen` first
- Session not active → deny

**Ordering guarantee**: The enforced sequence is always:
1. `attend listen` — restart listener, consume any pending narration
2. `attend clip` for each image — only allowed when listener is running
   and no narration is pending
3. Resume normal operation

If new narration arrives while consuming images, `attend clip` is blocked
until the agent runs `attend listen` again (picks up new narration, restarts
listener), then resumes `attend clip`.

`general_decision` gains a 6th parameter: `has_pending_clipboard: bool`.
When true, relation is Active, `has_pending` is false, and `receiver_alive`
is true, returns `Block(ClipboardReady)`. If receiver is not alive,
`StartReceiver` takes priority (the agent must restart the listener before
consuming clipboard images).

### GuidanceReason changes

`GuidanceReason::ClipboardReady` — a unit variant (no payload needed; the
blocking message is static since the agent just runs bare `attend clip`).

The agent-facing message:
```
Clipboard images pending. Run `attend clip` to view them.
```

No filenames in the blocking message — the agent doesn't need them to take
action. The `additionalContext` on the allow decision tells the agent which
image it's seeing after each successful `attend clip`.

### Render changes

**Agent delivery** (narration markdown sent via hooks):

`ClipboardContent::Image` renders as a placeholder with a short identifier:

```
🖼️ aebb33d9-9
```

The identifier is the first 10 characters of the UUID from the staging
filename (e.g., `2026-02-26T19-56-47.178389000Z-aebb33d9-96c3-...png` →
`aebb33d9-9`). The `additionalContext` on each `attend clip` call reports
the same identifier: "Delivering clipboard image aebb33." This lets the
agent correlate which image in the narration timeline it's seeing.

**Yank delivery** (markdown copied to system clipboard):

`ClipboardContent::Image` is rendered as a self-contained base64 data URI:

```markdown
![clipboard](data:image/png;base64,iVBORw0KGgo...)
```

The image is read from staging, base64-encoded, and embedded inline. This
makes the yanked markdown portable — the image travels with the text, no
external file references. The staging file is then archived as usual.

**Note on `ClipboardContent::Image { path }`**: The `path` field continues
to store the full absolute staging path. The 10-char UUID identifier is
derived from the filename at render time — no schema change needed.

### Narration protocol update

Add a section after the existing listener lifecycle documentation:

> **Clipboard images** — when narration contains 🖼️ placeholders (e.g.,
> `🖼️ aebb33`), you must view each one via `attend clip` before taking
> any other action. The sequence is always:
>
> 1. `attend listen` — always first. Restart the listener, consume any
>    pending narration. The listener must be running before anything else.
> 2. `attend clip` — run once per 🖼️ placeholder. Each call delivers one
>    image and reports its identifier. `attend clip` is only allowed when
>    the listener is running and no narration is pending.
> 3. Resume normal operation.
>
> If new narration arrives while you are consuming images, `attend clip`
> is blocked until you run `attend listen` again. Then resume `attend clip`.
>
> Run `attend clip` with no arguments — the hook handles everything.

### Cleanup changes

- `clipboard_staging_dir()` becomes session-scoped: takes `Option<&SessionId>`
  (same pattern as `browser_staging_dir`, `shell_staging_dir`)
- `clean.rs`: add `clipboard-staging/` cleanup alongside `archive/`,
  same retention cutoff. Reuse `clean_archive_dir` (walk session subdirs,
  prune by mtime).
- Remove the `Read` permission on `clipboard-staging/*` from install
- Add `Bash(attend clip)` permission in install
- Update uninstall to remove both old and new patterns

---

## Task breakdown (red-green TDD)

### Phase E1: Types + stubs (compiles)

| # | Task | Files |
|---|------|-------|
| 1 | Add `ClipboardReady` variant to `GuidanceReason` | `hook/types.rs` |
| 2 | Add `guidance_clipboard_ready.txt` message, add arm in `guidance_message` | `agent/messages/`, `agent/claude/output.rs` |
| 3 | Add `has_pending_clipboard` parameter to `general_decision` (pass `false` at all call sites) | `hook/decision.rs`, `hook.rs` |
| 4 | Detect `attend clip` commands in hook command parsing | `hook/command.rs` |
| 5 | Stub `attend clip` CLI subcommand (parse `--session` arg, no-op) | `cli.rs`, new `cli/clip.rs` |
| 6 | Make `clipboard_staging_dir` session-scoped | `narrate.rs` |
| 7 | Update daemon to pass session ID to clipboard staging | `narrate/clipboard_capture.rs`, `narrate/capture.rs`, `narrate/record.rs` |
| 8 | Stub yank image archival in `copy_yanked_to_clipboard` | `narrate/record.rs` |

### Phase E2: Tests (red)

| # | Task | Files |
|---|------|-------|
| 9 | Decision tests: `clipboard_ready_blocks_active`, `clipboard_ready_after_pending_narration`, `clipboard_ready_ignored_when_inactive`, `clipboard_ready_ignored_when_stolen` | `hook/tests/decision.rs` |
| 10 | Update all existing exhaustive decision tests for the new parameter (72→144 combinations) | `hook/tests/decision.rs` |
| 11 | Oracle model: add `pending_clipboard: [bool; NUM_SESSIONS]`, `WritePendingClipboard` / `ConsumeClipboard` ops, update `check_and_update` for ClipboardReady blocking and attend clip handling | `hook/tests/prop.rs` |
| 12 | Progress test: clipboard-ready blocks → `attend clip` clears → subsequent hook unblocked | `hook/tests/prop.rs` |
| 13 | Scenario tests: `clipboard_blocks_until_consumed`, `clip_allowed_when_clipboard_pending_and_listener_running`, `clip_blocked_by_pending_narration`, `clip_blocked_by_missing_listener`, `clip_PostToolUse_silent`, `multiple_images_consumed_one_at_a_time`, `clip_rewrites_command_with_session` | `hook/tests/scenario.rs` |
| 14 | Render tests: `clipboard_image_renders_as_placeholder_with_id`, `clipboard_image_yank_renders_as_base64`. Update existing `clipboard_image_renders_as_image_tag` from Phase 19C. | `narrate/merge/tests/render.rs` |
| 15 | CLI tests: `clip_serves_oldest_image`, `clip_archives_after_output`, `clip_empty_staging_is_noop` | `cli/clip.rs` |
| 16 | Yank tests: `yank_archives_clipboard_images`, `yank_embeds_base64_in_markdown` | `narrate/tests.rs` |

### Phase E3: Implementation (green)

| # | Task | Files |
|---|------|-------|
| 17 | Implement `general_decision` clipboard blocking logic (after `has_pending`, before `receiver_alive`) | `hook/decision.rs` |
| 18 | Compute `has_pending_clipboard` in `handle_general_hook` (check for `.png` files in `clipboard-staging/<session>/`) | `hook.rs` |
| 19 | Implement `attend clip` PreToolUse hook handler: NarrationReady check, session check, `updatedInput` rewrite, `additionalContext` with filename | `hook.rs`, `hook/command.rs` |
| 20 | Implement `attend clip --session` subcommand (oldest-first FIFO, base64 output, archive) | `cli/clip.rs` |
| 21 | Update render: `ClipboardContent::Image` → `🖼️ <id>` placeholder (agent), base64 data URI (yank) | `narrate/render.rs` |
| 22 | Implement yank image archival: move staging → archive, rewrite path in rendered markdown | `narrate/record.rs` |
| 23 | Update `clean.rs`: prune `clipboard-staging/<session>/` dirs by retention | `narrate/clean.rs` |

### Phase E4: Wiring + docs

| # | Task | Files |
|---|------|-------|
| 24 | Install: replace `Read(clipboard-staging/*)` with `Bash(attend clip)` | `agent/claude/settings/install.rs` |
| 25 | Uninstall: remove both old `Read` and new `Bash` patterns | `agent/claude/settings/uninstall.rs` |
| 26 | Update narration protocol: clipboard image lifecycle, ordering, blocking, `attend clip` | `agent/messages/narration_protocol.md` |
| 27 | Update setup guide if needed | `docs/setup.md` |

---

## Key files to modify

**Hook state machine:**
- `src/hook/types.rs` — `GuidanceReason::ClipboardReady`
- `src/hook/decision.rs` — `general_decision` with 6th parameter
- `src/hook.rs` — `handle_general_hook`, new clip handler
- `src/hook/command.rs` — detect `attend clip` commands

**Hook tests:**
- `src/hook/tests/decision.rs` — exhaustive decision tests (144 combinations)
- `src/hook/tests/prop.rs` — oracle model + progress test
- `src/hook/tests/scenario.rs` — integration scenarios
- `src/hook/tests/harness.rs` — clipboard staging helpers

**Agent output:**
- `src/agent/claude/output.rs` — `ClipboardReady` arm + `updatedInput`/`additionalContext` for clip allow
- `src/agent/messages/guidance_clipboard_ready.txt` — blocking message
- `src/agent/messages/narration_protocol.md` — clipboard image lifecycle

**CLI:**
- `src/cli.rs` — register `Clip` subcommand
- `src/cli/clip.rs` — new: `attend clip --session` implementation

**Narration pipeline:**
- `src/narrate.rs` — session-scoped `clipboard_staging_dir`
- `src/narrate/clipboard_capture.rs` — accept session-scoped staging dir
- `src/narrate/capture.rs` — pass session ID to clipboard thread
- `src/narrate/record.rs` — pass session ID, yank image archival
- `src/narrate/render.rs` — render image events as `🖼️ <id>` (agent) or base64 data URI (yank)
- `src/narrate/clean.rs` — session-scoped staging cleanup

**Permissions:**
- `src/agent/claude/settings/install.rs` — permission swap
- `src/agent/claude/settings/uninstall.rs` — permission cleanup

## Implementation notes

**Base64 image detection**: Claude Code's Bash tool renders `data:image/png;base64,...`
as an image ONLY when it is the sole stdout output AND the output is small.
Tested: a 100-byte 2x2 PNG (~100 chars base64) renders correctly. A 114KB
800x600 PNG (~150K chars base64) does NOT trigger detection — it's shown as
raw text and the command times out.

This means `attend clip` cannot deliver large images via base64 stdout.
**Fallback approach**: `attend clip` writes the image to a temp file and
outputs the path. The agent then uses the `Read` tool on that path. The
temp file path is session-scoped and ephemeral (deleted after read, or by
the next `attend clip` call). The PreToolUse hook can pre-authorize the
specific temp file path via `additionalContext` instructions, or we accept
that `Read` on a temp file will prompt the user (one-time allow).

Alternatively: `attend clip` writes to a well-known path like
`<cache>/attend/clip-output.png` (overwritten each call) and the
`additionalContext` tells the agent to `Read` that exact path. The install
step pre-authorizes `Read(<cache>/attend/clip-output.png)`.

**This is an open design question** — the base64-on-stdout approach only
works for tiny images. The plan should be updated once a delivery mechanism
for large images is confirmed.

**`updatedInput` JSON format**: For rewriting a Bash command, the PreToolUse
hook output needs:
```json
{
  "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow",
    "updatedInput": { "command": "attend clip --session <sid>" },
    "additionalContext": "<system-instruction>\nDelivering clipboard image aebb33d9-9.\n</system-instruction>"
  }
}
```
The current `render_decision` in `output.rs` doesn't support `updatedInput` —
it needs a new code path. The `attend_result` trait method and its callers may
need to pass through the rewritten command and context separately from the
`HookDecision` enum, or `HookDecision` gains a new variant for clip allow.

**Render mode**: `render_markdown` currently takes `(&[Event], SnipConfig)`.
For clipboard images, the agent path outputs `🖼️ <id>` but the yank path
needs to read the PNG file and base64-encode it. Two options:
1. Add a render mode parameter (`enum RenderMode { Agent, Yank }`)
2. Have the yank path post-process: call `render_markdown` normally (gets
   `🖼️ <id>`), then replace placeholders with base64 by reading staging files

Option 2 is fragile (regex on rendered output). Option 1 is cleaner.

**Command detection**: `detect_listen_command` returns `Option<ListenCommand>`
with `Listen` / `ListenStop` variants. Adding clip detection here means the
name doesn't fit. Either rename to `AttendCommand` with `Listen` / `ListenStop`
/ `Clip` variants, or add a separate `detect_clip_command`. The plan uses
the former approach but the implementer should choose based on code clarity.

**`capture::start` signature**: Already takes `clipboard_capture: bool` from
Phase 19. Now also needs `session_id: Option<&SessionId>` (or the staging
dir path directly) so the clipboard thread writes to the session-scoped dir.

**Phase 19 cleanup to revisit**: `record.rs` has a comment block (from da62fbc)
explaining that clipboard staging images are NOT cleaned up eagerly. This
comment and the `clean_flat_dir` call added to `clean.rs` both need updating —
`clean_flat_dir` becomes `clean_archive_dir` on the session-scoped
`clipboard-staging/` directory.

**Test harness**: `TestHarness` uses `CacheDirGuard` (from `state.rs`) to
redirect the cache directory to a temp dir. Clipboard staging dir functions
will automatically use this override. New test helpers (`write_pending_clipboard`,
`consume_clipboard`) should write/remove `.png` files in
`<temp_cache>/clipboard-staging/<session>/`.

## Existing code to reuse

- `crate::narrate::dir_key(Option<&SessionId>)` — session → dir key
- `crate::narrate::clean::clean_archive_dir` — walk session subdirs, prune by mtime
- `crate::hook::command::detect_listen_command` — pattern for detecting attend commands
- `crate::hook::decision::general_decision` — extend with new parameter
- `crate::hook::tests::harness::TestHarness` — filesystem-backed hook test infra
- `crate::agent::claude::output::render_decision` — pattern for hook JSON output (extend for `updatedInput`)

---

## Verification

1. `cargo fmt --check` + `cargo clippy` + `cargo test` — all clean
2. Manual (with session): record → copy image → stop → `attend listen` →
   see `🖼️ <id>` in narration → listener restarts → hook blocks with
   "Run `attend clip`" → run `attend clip` → see image in context +
   additionalContext reports matching ID → hook unblocks
3. Manual (multiple images): copy two images → verify `attend clip` must
   be run twice, one image per call, oldest first
4. Manual (yank, no session): record → copy image → yank → verify clipboard
   markdown has `![clipboard](data:image/png;base64,...)` inline
5. Manual: verify hook denies `attend clip` when narration is pending
   (must `attend listen` first)
6. Manual: verify hook denies `attend clip` when listener is not running
   (must `attend listen` first)
7. Verify all existing tests pass alongside new tests
