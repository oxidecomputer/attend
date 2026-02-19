# Extending attend

This document explains how to add support for a new editor or a new AI agent.

## Architecture overview

```
editor/        Reads state from editor backends (Zed, etc.)
  mod.rs         Merges results from all backends into QueryResult
  zed.rs         Zed-specific queries (SQLite)

agent/         Hook install/uninstall for each agent
  mod.rs         Agent trait, HookEvent enum, backend registry
  claude.rs      Claude Code settings.json manipulation

cli/           CLI definition (clap): verb-based subcommands
  mod.rs         Command enum, dispatch
  hook.rs        HookEvent subcommands with --agent flag
  narrate.rs     NarrateCommand subcommands

hook.rs        Hook runner: caching, change detection, output formatting

state.rs       Resolves raw byte offsets to line:col, reorders by recency
state/
  resolve.rs   Offset-to-position conversion, path relativization
```

Data flows in one direction:

```
editor backend  →  editor::query()  →  EditorState::build()  →  hook / CLI output
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
    fn query(&self) -> anyhow::Result<Option<QueryResult>> { ... }
}
```

Return `Ok(None)` when the editor isn't running or has no data. See
`src/editor/zed.rs` for a complete example using SQLite.

### `RawEditor` — one open tab

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
| `watch_paths()`         | no       | Filesystem paths to monitor for re-query         |
| `install_narration()`   | no       | Install narration hotkey/task integration        |
| `uninstall_narration()` | no       | Remove narration integration                     |
| `check_narration()`     | no       | Return diagnostic warnings (empty = healthy)     |

The narration methods are optional — return the default error/empty if the
editor doesn't support voice narration. See `src/editor/zed.rs` for a
complete implementation that installs Zed tasks and keybindings.

### 2. Register the backend — `src/editor/mod.rs`

Add the module and register it in the `EDITORS` slice:

```rust
mod zed;
mod <name>;
```

```rust
const EDITORS: &'static [&'static dyn Editor] = &[
    &zed::Zed,
    &<name>::Name,
];
```

That's it. Everything downstream (offset resolution, reordering, caching,
display) works automatically since it operates on the shared `QueryResult` type.

### Checklist

- [ ] `src/editor/<name>.rs` — `pub struct Name` + `impl Editor for Name`
- [ ] `src/editor/mod.rs` — `mod <name>;` declaration
- [ ] `src/editor/mod.rs` — add `&<name>::Name` to `EDITORS`

### Notes for VS Code / Cursor contributors

VS Code exposes editor state through its extension API rather than a local
database. A VS Code backend would likely:

1. Ship a lightweight VS Code extension that writes active editor state
   (file paths, byte-offset selections) to a known file or Unix socket.
2. Implement `query()` by reading that state file / connecting to the socket.
3. Implement `watch_paths()` to return the state file path for live updates.
4. For narration, register a keybinding in the extension's `package.json`
   that shells out to `attend narrate toggle`.

## Adding a new agent

An agent integration has two sides: **hook installation** (writing config into
the agent's settings) and **hook running** (the commands the agent invokes).

The `Agent` trait in `src/agent/mod.rs` covers both.

### 1. Create the agent module — `src/agent/<name>.rs`

Implement the `Agent` trait:

```rust
use super::{Agent, HookEvent};

pub struct Name;

impl Agent for Name {
    fn name(&self) -> &'static str { "<name>" }
    fn full_name(&self) -> &'static str { "<Name>" }

    fn run_hook(&self, event: HookEvent, cwd: Option<PathBuf>) -> anyhow::Result<()> {
        match event {
            HookEvent::UserPrompt => crate::hook::run(cwd),
            HookEvent::SessionStart => crate::hook::session_start(),
            HookEvent::Stop => crate::hook::stop(),
        }
    }

    fn install(&self, bin_cmd: &str, project: Option<PathBuf>) -> anyhow::Result<()> { ... }
    fn uninstall(&self, project: Option<PathBuf>) -> anyhow::Result<()> { ... }
}
```

`install()` writes hook commands (`{bin_cmd} hook --agent <name> user-prompt`, etc.)
into the agent's settings file. `uninstall()` removes them. See
`src/agent/claude.rs` for a complete example using JSON settings.

If the agent's hook protocol differs from the default (JSON on stdin with
`session_id` and `cwd`, text on stdout), implement custom handlers in
`run_hook()` instead of delegating to `hook::run()` / `hook::session_start()`.

### 2. Register the backend — `src/agent/mod.rs`

Add the module and register it in the `AGENTS` slice:

```rust
mod claude;
mod <name>;
```

```rust
pub const AGENTS: &'static [&'static dyn Agent] = &[
    &claude::Claude,
    &<name>::Name,
];
```

That's it. The CLI (`hook --agent <name> ...`, `install --agent <name>`, etc.)
is built automatically from the registered backends.

### Checklist

- [ ] `src/agent/<name>.rs` — `pub struct Name` + `impl Agent for Name`
- [ ] `src/agent/mod.rs` — `mod <name>;` declaration
- [ ] `src/agent/mod.rs` — add `&<name>::Name` to `AGENTS`
