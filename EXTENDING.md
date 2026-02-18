# Extending attend

This document explains how to add support for a new editor or a new AI agent.

## Architecture overview

```
editor/        Reads state from editor backends (Zed, etc.)
  mod.rs         Merges results from all backends into QueryResult
  zed.rs         Zed-specific queries (SQLite)

agent/         Hook install/uninstall for each agent
  mod.rs         Dispatches install/uninstall by Agent variant
  claude.rs      Claude Code settings.json manipulation

cli.rs         CLI definition (clap): flags, subcommands, dispatch

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

An editor backend reads open files and terminal state from whatever source the
editor exposes (database, socket, file, CLI) and returns a `QueryResult`.

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
- `terminals: Vec<PathBuf>` — working directories of active terminal tabs

### 2. Register the backend — `src/editor/mod.rs`

Add the module and register it in the `backends()` array:

```rust
mod zed;
mod <name>;
```

```rust
fn backends() -> &'static [&'static dyn Editor] {
    &[&zed::Zed, &<name>::Name]
}
```

That's it. Everything downstream (offset resolution, reordering, caching,
display) works automatically since it operates on the shared `QueryResult` type.

### Checklist

- [ ] `src/editor/<name>.rs` — `pub struct Name` + `impl Editor for Name`
- [ ] `src/editor/mod.rs` — `mod <name>;` declaration
- [ ] `src/editor/mod.rs` — add `&<name>::Name` to `backends()`

## Adding a new agent

An agent integration has two sides: **hook installation** (writing config into
the agent's settings) and **hook running** (the commands the agent invokes).

### 1. Add the CLI variant — `src/cli.rs`

Three changes, all marked with `// <--` comments:

```rust
pub enum Agent {
    Claude,
    <Name>,  // <-- Future agents go here
}
```

```rust
pub enum RunHook {
    #[command(subcommand)]
    Claude(ClaudeHook),
    #[command(subcommand)]
    <Name>(<Name>Hook),  // <-- Future agent hook runners go here
}
```

Define the hook event enum (what events the agent can trigger):

```rust
#[derive(Subcommand)]
pub enum <Name>Hook {
    UserPrompt,
    SessionStart,
}
```

Add dispatch arms in `Hook::run()`:

```rust
Hook::Run(RunHook::<Name>(<Name>Hook::UserPrompt)) => ...,
Hook::Run(RunHook::<Name>(<Name>Hook::SessionStart)) => ...,
// <-- Future agent hook dispatch goes here
```

### 2. Create the agent module — `src/agent/<name>.rs`

Implement `install()` and `uninstall()` functions that manipulate the agent's
settings file. See `src/agent/claude.rs` for the pattern.

### 3. Register the module — `src/agent/mod.rs`

Three changes, all marked with `// <--` comments:

```rust
mod claude;
mod <name>;  // <-- When adding an agent, add a module for it here
```

```rust
// inside install():
Agent::<Name> => <name>::install(&bin_cmd, project),
// <-- Install hooks for future agents go here
```

```rust
// inside uninstall():
Agent::<Name> => <name>::uninstall(project),
// <-- Uninstall hooks for future agents go here
```

### 4. Hook runner

If the new agent's hook protocol is similar to Claude Code's (JSON on stdin with
`session_id` and `cwd`, text on stdout), you can reuse `hook::run()` and
`hook::session_start()` directly.

If the agent has a different protocol, add a new runner in `src/hook.rs` or a
separate module, and wire it up in the `Hook::run()` dispatch (step 1).

### 5. Instructions file (optional)

If the agent needs custom instructions emitted at session start (like
`src/instructions.txt` for Claude), create a separate file and `include_str!()`
it from the session-start handler.

### Checklist

- [ ] `src/cli.rs` — `Agent` enum variant
- [ ] `src/cli.rs` — `RunHook` enum variant + hook event enum
- [ ] `src/cli.rs` — `Hook::run()` dispatch arms
- [ ] `src/agent/<name>.rs` — `install()` and `uninstall()`
- [ ] `src/agent/mod.rs` — `mod <name>;`, install/uninstall dispatch
- [ ] `src/hook.rs` — new runner if protocol differs, otherwise reuse existing
