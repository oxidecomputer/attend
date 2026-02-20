# Phase 3: Module Reorganization

**Dependencies**: Phase 2 (types settled, Config simplified — avoids reorg then re-edit).
**Effort**: Large | **Risk**: Low

This phase is pure reorganization — no logic changes. Each commit moves code without modifying it; logic changes are deferred to later phases.

---

## 3.1 `state.rs` split and rename

- Extract `atomic_write` -> `src/util.rs` (shared utility)
- Extract `cache_dir`, `listening_path`, `listening_session`, `version_path`, `installed_meta` -> `src/paths.rs` or `src/cache.rs`
- Rename remaining `state.rs` -> `src/editor_state.rs` (it's specifically about EditorState)

## 3.2 `json.rs` split

- `utc_now` + `Timestamped` -> replaced by chrono in Phase 4 (or move to `src/util.rs` temporarily)
- `CompactPayload` / `CompactFile` -> near the CLI consumer (`src/cli/` or stay)
- `ViewPayload` / `ViewFile` -> near the view consumer (`src/view/`)
- `split_selections` -> `src/view/` (only used there and in json)

## 3.3 `cli/mod.rs` split command defs from dispatch

- Command enum definitions stay in `cli/mod.rs` (or `cli/commands.rs`)
- Per-subcommand dispatch follows the `narrate.rs` pattern
- `cli/agent.rs`, `cli/view.rs`, `cli/watch.rs`, etc.
- Use `#[command(flatten)]` for single-arg variants

## 3.4 `narrate/mod.rs` barrel module

- `bench()` -> `narrate/transcribe/` (it's about transcription engines)
- `status()` -> `narrate/status.rs`
- `clean()` + `clean_archive_dir()` -> `narrate/clean.rs`
- What's left: path definitions, `process_alive()`, `resolve_session()`, submodule declarations

## 3.5 `narrate/capture.rs` split

- Editor state polling thread -> `narrate/editor_capture.rs` (or similar)
- File diff tracking thread -> `narrate/diff_capture.rs`
- Shared `CaptureEvents` handle stays or gets its own file

## 3.6 `editor/zed.rs` submodule directory

- `editor/zed/mod.rs` — trait impl, `Zed` struct
- `editor/zed/db.rs` — `find_db()`, database queries, `query_editors()`
- `editor/zed/keybindings.rs` — install/uninstall keybindings (JSONC manipulation)
- `editor/zed/tasks.rs` — install/uninstall tasks (JSONC manipulation)
- `editor/zed/health.rs` — `check_narration()`, `is_narration_keybinding()`

## 3.7 `merge.rs` extract `render_markdown`

- Move `render_markdown` + `SnipConfig` to `narrate/render.rs` (presentation concern, not merging)
- `merge.rs` retains only event compression, sorting, and diff merging

## 3.8 `watch.rs` split

- Terminal helpers (`clear_screen`, `fit_to_terminal`) -> `src/terminal.rs`
- Format-specific rendering logic -> cleaner match arms or helper functions
- Consider `crossterm` dependency for terminal handling

## 3.9 `editor/mod.rs` cleanup

- Remove `watch_paths()` default method (dead code, polling approach)
- Remove `all_watch_paths()` function
- Move `EDITORS` registry to top of file for visibility

## 3.10 Future-proof editor trait for line:col backends

- Note: Zed gives byte offsets; other editors may give line:col instead
- Add a comment or design note in the editor trait about a future normalization layer
- No implementation needed now, but the trait should not assume byte offsets in its contract

---

## Verification

- All tests pass (proves semantic equivalence)
- No file in `src/` exceeds a reasonable size (use judgment, but flag anything over ~300 lines as worth reviewing)
- `grep -rn 'pub(crate)' src/` — check that visibility didn't accidentally widen; items that were `pub(crate)` should remain so unless there's a reason
- Each commit moves code without modifying it; logic changes are deferred to later phases
