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
  messages/        Shared message templates (protocol descriptions, guidance)
  claude/          Claude Code agent backend
    mod.rs           Agent trait impl (delegates to submodules)
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

    fn query(&self) -> anyhow::Result<Option<QueryResult>> {
        // Return open tabs with byte-offset selections.
        // Return Ok(None) when the editor isn't running or has no data.
        Ok(None)
    }

    fn install_narration(&self, bin_cmd: &str) -> anyhow::Result<()> {
        // Register hotkeys/tasks that run `{bin_cmd} narrate toggle` and
        // `{bin_cmd} narrate start`. Zed installs tasks.json entries +
        // keymap.json bindings.
        Ok(())
    }

    fn uninstall_narration(&self) -> anyhow::Result<()> {
        // Remove whatever install_narration() added.
        Ok(())
    }

    fn check_narration(&self) -> anyhow::Result<Vec<String>> {
        // Return diagnostic warnings. Empty vec = healthy.
        // Zed checks for stale task paths and missing keybindings.
        Ok(Vec::new())
    }
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
`hook/` that handles session lifecycle, caching, and narration delivery) and
**agent-specific rendering** (how each agent parses input and formats output).

The `Agent` trait in `src/agent/mod.rs` covers the agent-specific side. The
shared orchestration lives in `hook/` and calls into the trait.

### How hooks work

When you run `attend install --agent <agent>`, hooks are installed into the
agent's settings. Five hook events drive the integration:

| Hook              | When it fires                    | What the orchestrator does                                       |
|-------------------|----------------------------------|------------------------------------------------------------------|
| `SessionStart`    | Session start, clear, compact    | Clear per-session cache, auto-upgrade hooks, emit instructions   |
| `UserPromptSubmit`| Before each user prompt          | Detect `/attend`, or query + deduplicate + emit editor context   |
| `Stop`            | Session stops                    | Deliver pending narration or guidance                            |
| `PreToolUse`      | Before each tool call            | Deliver pending narration between tools within a response        |
| `PostToolUse`     | After each tool call             | Deliver pending narration between tools within a response        |

The orchestrator (`hook/`) calls `agent.parse_hook_input()` to get a
`HookInput`, then calls the appropriate agent output method based on the
event. The agent never sees raw hook logic — it just parses input and
renders output.

### `HookInput` is parsed from agent-specific sources

```rust
pub struct HookInput {
    pub session_id: Option<SessionId>,
    pub cwd: Option<Utf8PathBuf>,
    pub kind: HookKind,
}

pub enum HookKind {
    SessionStart,
    UserPrompt { prompt: Option<String> },
    Stop { stop_hook_active: bool },
    ToolUse { bash_command: Option<String> },
}
```

Claude reads this from JSON on stdin. Other agents might read environment
variables, a socket, or a config file.

### `HookDecision` describes semantic outcomes

The hook orchestrator computes a semantic decision that each agent renders
in its own format:

| Variant                    | Meaning                                                  |
|----------------------------|----------------------------------------------------------|
| `Silent`                   | No output needed                                         |
| `Guidance(reason, effect)` | Operational guidance with block or approve effect        |

Narration content is delivered separately via `Agent::deliver_narration()`,
not through `HookDecision`. The orchestrator calls `deliver_narration()` when
pending narration is found during an `attend listen` PreToolUse hook.

Guidance reasons:

| Reason                  | Meaning                                            |
|-------------------------|----------------------------------------------------|
| `SessionMoved`          | Narration is active in a different session         |
| `StartReceiver`         | No receiver running: agent should start one        |
| `NarrationReady`        | Pending narration: agent should run `attend listen`|
| `ListenerAlreadyActive` | A listener is already running for this session     |
| `ListenerStarted`       | A listener was just started in the background      |

### 1. Create the agent module — `src/agent/<name>/`

Create a directory with at least a `mod.rs` implementing the `Agent` trait:

```rust
use camino::Utf8PathBuf;

use super::Agent;
use crate::hook::{HookDecision, HookInput, HookType};
use crate::state::{EditorState, SessionId};

pub struct Name;

impl Agent for Name {
    fn name(&self) -> &'static str { "<name>" }

    // --- Input ---

    fn parse_hook_input(&self, hook_type: HookType) -> HookInput {
        // Read from whatever source this agent provides (stdin, env, etc.)
        // Return HookInput with session_id, cwd, and hook-type-specific kind.
        HookInput::default()
    }

    // --- Output ---

    fn session_start(&self, input: &HookInput, is_listening: bool) -> anyhow::Result<()> {
        // Emit instructions for the agent session.
        // If is_listening, also emit narration instructions.
        Ok(())
    }

    fn editor_context(&self, state: &EditorState) -> anyhow::Result<()> {
        // Render editor state (open files, cursors, selections) to stdout.
        Ok(())
    }

    fn attend_activate(&self, session_id: &SessionId) -> anyhow::Result<()> {
        // Acknowledge narration activation to stdout.
        Ok(())
    }

    fn attend_result(&self, decision: &HookDecision, hook_type: HookType) -> anyhow::Result<()> {
        // Render the hook decision to stdout. hook_type controls whether
        // guidance should block or approve (e.g., PreToolUse approves
        // StartReceiver rather than blocking).
        Ok(())
    }

    // --- Install/Uninstall ---

    fn install(&self, bin_cmd: &str, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        // Write hook commands and a skill explaining what to do into the agent's settings.
        // Commands: {bin_cmd} hook session-start --agent <name>
        //           {bin_cmd} hook user-prompt --agent <name>
        //           {bin_cmd} hook stop --agent <name>
        //           {bin_cmd} hook pre-tool-use --agent <name>
        //           {bin_cmd} hook post-tool-use --agent <name>
        Ok(())
    }

    fn uninstall(&self, project: Option<Utf8PathBuf>) -> anyhow::Result<()> {
        // Remove hook entries and skill from the agent's settings.
        Ok(())
    }
}
```

See `src/agent/claude/` for a complete example. Key implementation notes:

- **Input**: Claude reads JSON from stdin (`{ "session_id": "...", "cwd": "...", "prompt": "..." }`). Other agents will have different input sources.
- **Output**: Claude emits JSON responses to stdout (`{ "decision": "block", "reason": "..." }`). Other agents may use entirely different formats.
- **Install**: Claude writes to `~/.claude/settings.json` (global) or `.claude/settings.local.json` (project). It also installs a SKILL.md file and pre-authorizes tool permissions. Other agents will have settings elsewhere.
- **Idempotency**: `install()` must be safe to call repeatedly. Remove existing entries before adding new ones so the binary path stays current.
- **Non-interference**: Installation and uninstallation must not interfere with other settings and tools.

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

### 3. Shared message templates

Most message templates live in `src/agent/messages/` and are shared across
all agents. Use `include_str!` to embed them:

| Template | Purpose | Placeholders |
|----------|---------|--------------|
| `editor_context_instructions.txt` | How to interpret `<editor-context>` tags and use `attend look` | `{bin_cmd}` |
| `narration_protocol.md` | Full narration protocol: silence requirement, two delivery paths, receiver restart behavior, `<narration>` tag format, cursor-only handling, `include_dirs` | None |
| `narration_pause.txt` | "Pause and consider narration before using tools" | None |
| `activate_response.txt` | Confirmation when narration is activated | None |
| `guidance_session_moved.txt` | "Narration moved to another session" | None |
| `guidance_start_receiver.txt` | "Start the receiver" nudge | None |
| `guidance_listener_active.txt` | "Listener already running" | None |

These cover the attend protocol — what narration is, how to behave, what
operational messages mean. Your agent gets all of this for free.

Agent-specific templates go in `src/agent/<name>/messages/`. Claude keeps
two files there:

- `skill_frontmatter.md` — YAML metadata for Claude Code's skill system
- `skill_body.md` — Claude-specific activation instructions (how to run the
  listener in the background, tool description hints) plus a
  `{narration_protocol}` placeholder that pulls in the shared protocol

Your agent may want to use different content because of differences in how its
execution harness operates.

### 4. Agent-specific instructions

Agents need instructions that teach them how to interact with attend. The shared
templates handle protocol-level content. Your agent adds mechanism-specific
content explaining how to actually execute commands in its environment.

At minimum, your agent should:

1. **On session start**: emit `editor_context_instructions.txt` (formatted
   with `bin_cmd`) so the agent knows how to interpret editor context.
2. **On narration activation**: emit `activate_response.txt` so the agent
   knows to start listening. Include your own activation instructions
   explaining how to run `attend listen` in your agent's execution model.
3. **On narration re-emission** (session start with `is_listening = true`):
   re-emit narration instructions so the agent restarts the receiver after
   context compaction or clear.

### Checklist

- [ ] `src/agent/<name>/mod.rs` — `pub struct Name` + `impl Agent for Name`
- [ ] `src/agent/mod.rs` — `mod <name>;` declaration
- [ ] `src/agent/mod.rs` — add `&<name>::Name` to `AGENTS`
- [ ] Use shared templates from `src/agent/messages/` for protocol content
- [ ] Add agent-specific templates for activation and execution instructions
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

1. **Hook delivery** (non-blocking): The Stop, PreToolUse, and PostToolUse
   hooks collect pending narration files, render them as markdown wrapped in
   `<narration>` tags, and deliver via `attend_result(PendingNarration)`.
   PreToolUse and PostToolUse ensure narration arrives between tools within
   a single response, not just at the end.

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

### Receiver output protocol

The `attend listen` receiver is agent-agnostic. It uses a standard output
protocol based on XML tags:

- Narration content is wrapped in `<narration>` tags
- Operational instructions (restart, conflict) are wrapped in
  `<system-instruction>` tags

Each agent's instructions teach its LLM to expect this format. If an agent's
LLM requires fundamentally different framing, it can implement a custom
listener, but the default protocol works well for LLMs that handle XML tags.
