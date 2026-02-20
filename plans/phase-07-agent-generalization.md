# Phase 7: Agent Trait Generalization

**Dependencies**: Phase 3 (modules reorged), Phase 5 (error handling clean).
**Effort**: Medium | **Risk**: Medium
**Status**: Done (7.1–7.3 complete, 7.4 deferred)

---

## Architecture

Three-layer separation between agent-specific and shared concerns:

1. **Input** (agent-specific): `parse_hook_input()` returns `HookInput`. Claude reads stdin JSON; another agent might use env vars, files, etc.
2. **Logic** (shared, in `hook.rs`): Cache management, auto-upgrade, editor state capture/dedup, stop decision, attend activation (writing the listening file).
3. **Output** (agent-specific): One render method per hook. Receives computed results, produces output in agent-specific format.

## 7.1 Refactor hook logic into generic + agent-specific

### New types in `src/hook.rs`

```rust
/// Parsed input from an agent hook invocation.
pub struct HookInput {
    pub session_id: Option<SessionId>,
    pub cwd: Option<Utf8PathBuf>,
    pub prompt: Option<String>,       // UserPrompt only
    pub stop_hook_active: bool,       // Stop only
}

/// Structured stop decision with semantic variants.
pub enum StopDecision {
    Silent,
    SessionMoved,
    PendingNarration { content: String },
    StartReceiver,
}
```

### Agent trait (`src/agent/mod.rs`)

Removed `HookEvent` enum and `run_hook` method. New trait:

```rust
pub trait Agent: Sync {
    fn name(&self) -> &'static str;
    fn parse_hook_input(&self) -> HookInput;
    fn session_start(&self, input: &HookInput, is_listening: bool) -> anyhow::Result<()>;
    fn editor_context(&self, state: &EditorState) -> anyhow::Result<()>;
    fn attend_activate(&self, session_id: &SessionId) -> anyhow::Result<()>;
    fn attend_result(&self, decision: &StopDecision) -> anyhow::Result<()>;
    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()>;
    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()>;
}
```

`HookInput` re-exported from `agent/mod.rs` for convenience.

### Shared orchestrators in `src/hook.rs`

Each takes `&dyn Agent` and calls agent methods for I/O:

- **`session_start(agent)`**: parse input → delete session cache → auto-upgrade → compute `is_listening` → `agent.session_start()`
- **`user_prompt(agent, cli_cwd)`**: parse input → check `/attend` → if yes: write listening file, `agent.attend_activate()` → else: capture editor state → dedup → if changed: `agent.editor_context()`
- **`stop(agent)`**: parse input → resolve pending narration → compute `StopDecision` → archive if `PendingNarration` → `agent.attend_result()`

### Claude impl (`src/agent/claude.rs`)

Moved from `hook.rs`:
- `read_stdin_json()` → `parse_hook_input()`: reads stdin JSON, extracts all `HookInput` fields
- `narration_instructions()`: builds `<narration-instructions>` wrapper

Render methods:
- `session_start()`: prints `instructions.txt` (templated with bin path); if listening, also prints narration instructions
- `editor_context()`: prints `<editor-context>` tags
- `attend_activate()`: prints JSON `{"additionalContext": "..."}`
- `attend_result()`: maps `StopDecision` variants → JSON `{"decision": "approve"|"block", "reason": "..."}`

### CLI dispatch (`src/cli/hook.rs`)

Dispatches directly to `hook::*` orchestrators with resolved `&dyn Agent`, removing the intermediate `agent::HookEvent` indirection.

### Tests (`src/hook/tests.rs`)

- `stop_decision` tests use new enum variants (`SessionMoved`, `PendingNarration`, `StartReceiver`) instead of string matching on `Approve`/`Block`.
- `is_attend_prompt` tests use `HookInput` structs instead of raw JSON values.

## 7.2 Split narration instructions

Done by architecture: instructions are agent-specific in Claude's `session_start()` render method. `instructions.txt` (generic editor-context docs) and `claude_skill_body.md` / `claude_skill_frontmatter.md` stay in their existing locations. No separate shared protocol template needed until a second agent exists.

## 7.3 Track project-specific installations

### `src/state.rs`

Added field to `InstallMeta`:
```rust
#[serde(default)]
pub project_paths: Vec<Utf8PathBuf>,
```

### `src/cli/install.rs`

- `install()`: if `--project` given, appends path to `project_paths` (deduplicated). Preserves existing paths from prior installs.
- `uninstall()` without `--project`: iterates `project_paths`, uninstalls each (best-effort), then clears the list before proceeding with global uninstall.

### `src/hook.rs`

`auto_upgrade_hooks()` preserves `project_paths` through version upgrades.

## 7.4 Research skill format generalization

Deferred — no second agent to target yet. The trait boundary from 7.1 is the prerequisite; implementing a new agent requires only the trait methods, not touching `hook.rs`.

---

## Files modified

| File | Changes |
|------|---------|
| `src/agent/mod.rs` | New trait methods, removed `run_hook`/`HookEvent`, `HookInput` re-export |
| `src/agent/claude.rs` | Implemented new trait methods, moved stdin parsing + `narration_instructions` here |
| `src/hook.rs` | Public `HookInput`/`StopDecision`, orchestrators take `&dyn Agent`, removed `read_stdin_json`/`handle_attend_activate`/`narration_instructions` |
| `src/hook/tests.rs` | Tests use `StopDecision` variants and `HookInput` structs |
| `src/cli/hook.rs` | Direct dispatch to `hook::*` functions, removed `agent::HookEvent` usage |
| `src/state.rs` | Added `project_paths` to `InstallMeta` |
| `src/cli/install.rs` | Track/clean project paths on install/uninstall |
