# Extending attend

This document explains how to add support for a new editor or a new AI agent.

## Architecture overview

```
editor/          Reads state from editor backends (Zed, etc.)
  mod.rs           Editor trait, merges results from all backends into QueryResult
  zed/             Zed backend (submodule directory)
    mod.rs           Query (SQLite), narration install, health checks
    ...

agent/           Hook installation and output rendering for each agent
  mod.rs           Agent trait, backend registry, resolve_bin_cmd
  claude.rs        Claude Code: settings.json hooks, SKILL.md, permissions
  claude/          Claude-specific assets
    ...
```

## Adding a new editor

An editor backend reads open files from whatever source the editor exposes
(database, socket, file, CLI) and returns a `QueryResult`.

### 1. Create the module — `src/editor/<name>.rs`

Implement the `Editor` trait:

```rust
use super::{Editor, QueryResult, RawEditor};

pub struct Name;

impl Editor for Name {
    fn name(&self) -> &'static str { "<name>" }
    fn query(&self) -> anyhow::Result<Option<QueryResult>> { ... }
}
```

Return `Ok(None)` when the editor isn't running or has no data. See
`src/editor/zed/mod.rs` for a complete example using SQLite.

### `RawEditor` represents one open tab/pane with a single cursor or selection

| Field       | Type            | Meaning                                    |
|-------------|-----------------|--------------------------------------------|
| `path`      | `PathBuf`       | Absolute file path                         |
| `sel_start` | `Option<i64>`   | Byte offset of selection/cursor start      |
| `sel_end`   | `Option<i64>`   | Byte offset of selection/cursor end        |

A cursor is represented as `sel_start == sel_end`. Return `None` for both
when the editor doesn't expose selection data.

### `Editor` trait methods

| Method                  | Required | Purpose                                          |
|-------------------------|----------|--------------------------------------------------|
| `name()`                | yes      | CLI name, e.g. `"zed"`                           |
| `query()`               | yes      | Return open tabs with byte-offset selections     |
| `install_narration()`   | no       | Install narration hotkey/task integration        |
| `uninstall_narration()` | no       | Remove narration integration                     |
| `check_narration()`     | no       | Return diagnostic warnings (empty = healthy)     |

The narration methods are optional; they return the default error/empty if the
editor doesn't support voice narration. See `src/editor/zed/` for a complete
implementation that installs Zed tasks and keybindings.

### 2. Register the backend in `src/editor/mod.rs`

Add the module and register it in the `EDITORS` slice:

```rust
mod zed;
mod <name>;
```

```rust
pub const EDITORS: &[&'static dyn Editor] = &[
    &zed::Zed,
    &<name>::<Name>,
];
```

That's it. Everything downstream (offset resolution, reordering, caching,
display) works automatically since it operates on the shared `QueryResult` type.

### Checklist

- [ ] `src/editor/<name>.rs` — `pub struct Name` + `impl Editor for Name`
- [ ] `src/editor/mod.rs` — `mod <name>;` declaration
- [ ] `src/editor/mod.rs` — add `&<name>::Name` to `EDITORS`

### Notes for future VS Code / Cursor contributors

VS Code exposes editor state through its extension API rather than a local
database. A VS Code backend would likely:

1. Ship a lightweight VS Code extension that writes active editor state
   (file paths, byte-offset selections) to a known file or Unix socket.
2. Implement `query()` by reading that state file / connecting to the socket.
3. For narration, register a keybinding in the extension's `package.json`
   that shells out to `attend narrate toggle`.

## Adding a new agent

An agent integration has two sides: **hook orchestration** (shared logic in
`hook.rs` that handles session lifecycle, caching, and narration delivery) and
**agent-specific rendering** (how each agent parses input and formats output).

The `Agent` trait in `src/agent/mod.rs` covers the agent-specific side. The
shared orchestration lives in `hook.rs` and calls into the trait.

### How hooks work

When you run `attend install --agent <agent>`, it's meant to install three hooks
into the agent's settings:

| Hook              | When it fires                    | What the orchestrator does                                       |
|-------------------|----------------------------------|------------------------------------------------------------------|
| `SessionStart`    | Session start, clear, compact    | Clear per-session cache, auto-upgrade hooks, emit instructions   |
| `UserPromptSubmit`| Before each user prompt          | Detect `/attend`, or query + deduplicate + emit editor context   |
| `Stop`            | Session stops or tool completes  | Compute stop decision, deliver pending narration                 |

The orchestrator (`hook.rs`) calls `agent.parse_hook_input()` to get a
`HookInput`, then calls the appropriate agent output method based on the
event. The agent never sees raw hook logic — it just parses input and
renders output.

### `HookInput` is parsed from agent-specific sources

```rust
pub struct HookInput {
    pub session_id: Option<SessionId>,
    pub cwd: Option<Utf8PathBuf>,
    pub prompt: Option<String>,       // UserPrompt hook only
    pub stop_hook_active: bool,       // Re-invocation after a previous block
}
```

Claude reads this from JSON on stdin. Other agents might read environment
variables, a socket, or a config file.

### `StopDecision` described semantic outcomes from a stop hook

The stop hook doesn't hardcode textual output. It computes a semantic decision
that each agent renders in its own format:

| Variant             | Meaning                                              |
|---------------------|------------------------------------------------------|
| `Silent`            | No output needed                                     |
| `SessionMoved`      | Narration is active in a different session           |
| `PendingNarration`  | Narration content ready to deliver                   |
| `StartReceiver`     | No receiver running: agent should start one          |

### 1. Create the agent module — `src/agent/<name>.rs`

Implement the `Agent` trait:

```rust
use camino::Utf8PathBuf;

use super::Agent;
use crate::hook::{HookInput, StopDecision};
use crate::state::{EditorState, SessionId};

pub struct Name;

impl Agent for Name {
    fn name(&self) -> &'static str { "<name>" }

    // --- Input ---

    fn parse_hook_input(&self) -> HookInput {
        // Read from whatever source this agent provides (stdin, env, etc.)
        // Return HookInput with session_id, cwd, prompt, stop_hook_active.
        HookInput::default()
    }

    // --- Output ---

    fn session_start(&self, input: &HookInput, is_listening: bool) -> anyhow::Result<()> {
        // Emit instructions for the agent session.
        // If is_listening, also emit narration skill instructions.
        Ok(())
    }

    fn editor_context(&self, state: &EditorState) -> anyhow::Result<()> {
        // Render editor state (open files, cursors, selections) in an
        // agent-specific manner (i.e. to stdout).
        Ok(())
    }

    fn attend_activate(&self, session_id: &SessionId) -> anyhow::Result<()> {
        // Acknowledge /attend activation in an agent-specific manner (i.e. to
        // stdout).
        Ok(())
    }

    fn attend_result(&self, decision: &StopDecision) -> anyhow::Result<()> {
        // Output the stop decision in agent-specific manner (i.e. to stdout).
        Ok(())
    }

    // --- Install/Uninstall ---

    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        // Write hook commands into the agent's settings.
        // Commands are: {bin_cmd} hook --agent <name> session-start
        //               {bin_cmd} hook --agent <name> user-prompt
        //               {bin_cmd} hook --agent <name> stop
        Ok(())
    }

    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        // Remove hook entries from the agent's settings.
        Ok(())
    }
}
```

See `src/agent/claude.rs` for a complete example. Key implementation notes:

- **Input**: Claude reads JSON from stdin (`{ "session_id": "...", "cwd": "...", "prompt": "..." }`). Other agents will have different input sources.
- **Output**: Different agent harnesses might have different ways to send output from a hook to the agent. The agent captures it and presents it to the user. Claude uses `<system-reminder>` XML tags and JSON responses via stdout, but others may use entirely different formats or interfaces.
- **Install**: Claude writes to `~/.claude/settings.json` (global) or `.claude/settings.local.json` (project). It also installs a SKILL.md file for discoverability and pre-authorizes tool permissions. Other tools will have settings elsewhere.
- **Idempotency**: `install()` must be safe to call repeatedly. Remove existing entries before adding new ones so the binary path stays current.

### 2. Register the backend — `src/agent/mod.rs`

Add the module and register it in the `AGENTS` slice:

```rust
mod claude;
mod <name>;
```

```rust
pub const AGENTS: &[&'static dyn Agent] = &[
    &claude::Claude,
    &<name>::Name,
];
```

The CLI (`hook --agent <name> ...`, `install --agent <name>`, etc.)
is built automatically from the registered backends.

### 3. Agent-specific instructions (optional)

Agents often need instructions that teach them how to interact with attend.
Claude uses three layers:

1. **Session-start instructions** (`src/instructions.txt`): Emitted on every
   session start. Explains the `<editor-context>` format and the `attend look`
   command. These are intended to be agent-agnostic, but you may emit other
   instructions if you need to.

2. **Skill file** (frontmatter + body): Installed to the agent's skill
   directory for discoverability. Declares allowed tools and explains how to
   activate and use narration mode.

3. **Narration re-emission**: When `is_listening` is true on session start
   (after context compaction), re-emit narration instructions so the agent
   knows to restart its background receiver.

Other agents may not need all three layers, but should at minimum emit
instructions on session start explaining how to interpret editor context.

### Checklist

- [ ] `src/agent/<name>.rs` — `pub struct Name` + `impl Agent for Name`
- [ ] `src/agent/mod.rs` — `mod <name>;` declaration
- [ ] `src/agent/mod.rs` — add `&<name>::Name` to `AGENTS`
- [ ] Instructions template for the agent (optional but recommended)
- [ ] Test hook install/uninstall round-trips cleanly

## Supporting infrastructure

### Auto-upgrade

On each `SessionStart` hook, `attend` checks whether the running binary version
matches the version that installed the hooks (`~/.cache/attend/version.json`).
On mismatch, it automatically reinstalls all previously registered agents and
editors. This ensures hooks stay compatible after `cargo install` updates.

### Project path tracking

`attend install --project /path/to/project` records the path in
`InstallMeta.project_paths`. On `attend uninstall` (without `--project`),
all tracked project paths are cleaned up. This prevents stale
project-local config from accumulating.

### Narration delivery

Narration reaches the agent through two paths:

1. **Stop hook** (non-blocking): When the session stops, the stop hook
   collects pending narration files, renders them as markdown wrapped in
   `<narration>` tags, and delivers via `attend_result(PendingNarration)`.

2. **Background receiver** (blocking): When `attend_result(StartReceiver)`
   fires, the agent starts `attend listen` in the background. The receiver
   polls for pending files and prints them when they arrive, then exits so
   the agent can restart it for the next narration. This means that arriving
   narration can **prompt** a new conversational turn for the agent; without
   this mechanism, only narration *during* the agent's turn would register
   without manual intervention.

Both paths filter narration context to the project scope (cwd + `include_dirs`)
and relativize paths before delivery, so that there is no leak of file contents
from outside the agent's permissioned path.
