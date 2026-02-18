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

cli.rs         CLI definition (clap): flags, subcommands, dynamic RunHook dispatch

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

The `QueryResult` contains:
- `editors: Vec<RawEditor>` — each with an absolute `PathBuf` and optional
  byte-offset selection start/end

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

## Adding a new agent

An agent integration has two sides: **hook installation** (writing config into
the agent's settings) and **hook running** (the commands the agent invokes).

The `Agent` trait in `src/agent/mod.rs` covers both. CLI subcommands and
dispatch are built dynamically from registered backends — no `cli.rs` changes
needed.

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
        }
    }

    fn install(&self, bin_cmd: &str, project: Option<PathBuf>) -> anyhow::Result<()> { ... }
    fn uninstall(&self, project: Option<PathBuf>) -> anyhow::Result<()> { ... }
}
```

`install()` writes hook commands (`{bin_cmd} hook run <name> user-prompt`, etc.)
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

That's it. The CLI (`hook run <name> ...`, `hook install --agent <name>`, etc.)
is built automatically from the registered backends.

### Checklist

- [ ] `src/agent/<name>.rs` — `pub struct Name` + `impl Agent for Name`
- [ ] `src/agent/mod.rs` — `mod <name>;` declaration
- [ ] `src/agent/mod.rs` — add `&<name>::Name` to `AGENTS`
