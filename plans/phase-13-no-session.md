# Phase 13: No-Session Support

**Dependencies**: Phase 12b (browser staging pattern).
**Effort**: Small | **Risk**: Low

---

## Motivation

Today, staging directories (`browser_staging_dir`, and the future
`shell_staging_dir` from phase 15) require a `SessionId` for their path.
Without an agent session, browser and shell events have nowhere to go. The
daemon's pending output also falls back to a single `narration.json` file
rather than the standard `pending/<id>/` directory layout.

This means narration without an agent listening is second-class: core capture
(audio, editor, ext selections) works, but browser selections and shell
commands (phase 15) are silently dropped.

**Fix**: Use a well-known `"_local"` fallback directory when no session exists,
so the full capture pipeline works regardless of whether an agent is listening.
This is a prerequisite for phase 15 (shell hooks, which add another staging
directory) and phase 16 (yank-to-clipboard, which enables agent-free
narration).

---

## Design

### `_local` fallback directory

Staging and pending directory functions currently take `&SessionId`. Change
them to accept `Option<&SessionId>` and fall back to `"_local"`:

```
~/.cache/attend/browser-staging/_local/<timestamp>.json
~/.cache/attend/shell-staging/_local/<timestamp>.json   (phase 15)
~/.cache/attend/pending/_local/<timestamp>.json
```

This replaces the current `narration.json` single-file fallback for the
no-session case, unifying the directory layout: session or not, narration is
always in `pending/<id>/`.

### Critical gate unchanged

The `record.lock` file (daemon is running) remains the sole gate for whether
events are captured at all. Browser bridge and shell hooks already check
`record_lock_path().exists()` as their first fast-path guard and exit
immediately if it's absent. This does not change. The `_local` fallback only
affects *where* events are staged when the daemon is running but no agent
session exists — it does not cause passive capture when narration is inactive.

### Browser bridge and shell hook changes

These currently also check `listening_session()` and bail if no session.
Change them to proceed with the `"_local"` staging directory when
`listening_session()` returns `None` (but `record.lock` exists). This gives
full capture functionality during active narration regardless of whether an
agent is listening.

### Cleanup

`_local` staging files are cleaned up on stop just like session-keyed files.
No special retention policy needed.

---

## Task breakdown

| # | Task | Files |
|---|------|-------|
| 1 | Staging/pending dir functions: accept `Option<&SessionId>`, fall back to `"_local"` | `narrate.rs` |
| 2 | Daemon: pass `Option` to pending/staging dirs, remove `narration.json` fallback | `narrate/record.rs` |
| 3 | Browser bridge: use `_local` when no session but `record.lock` exists | `cli/browser_bridge.rs` |
| 4 | Receive: `collect_pending` / `read_pending` handle `_local` directory | `narrate/receive.rs` |
| 5 | Tests: no-session staging, pending read from `_local`, browser bridge fallback | tests |

---

## Verification

- Start recording without any `attend listen` session active.
- Verify browser selections are staged to `browser-staging/_local/`.
- Stop recording; verify output is in `pending/_local/`.
- `attend narrate status` shows pending count from `_local` directory.
