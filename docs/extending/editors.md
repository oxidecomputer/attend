# How to add a new editor

An editor backend reads open files from whatever source the editor exposes
(database, socket, file, CLI) and returns a `QueryResult`. See [extending
reference](reference.md#editor-trait) for the full trait API.

## 1. Create the module

Create `src/editor/<name>.rs` implementing the `Editor` trait:

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
        // Register hotkeys/tasks that run `{bin_cmd} narrate toggle`, etc.
        Ok(())
    }

    fn uninstall_narration(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn check_narration(&self) -> anyhow::Result<Vec<String>> {
        // Return diagnostic warnings. Empty vec = healthy.
        Ok(Vec::new())
    }
}
```

Return `Ok(None)` when the editor isn't running or has no data. See
`src/editor/zed/mod.rs` for a complete example using SQLite.

## 2. Register the backend

In `src/editor/mod.rs`, add the module and register it:

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

Everything downstream (offset resolution, reordering, caching, display) works
automatically since it operates on the shared `QueryResult` type.

## Checklist

- [ ] `src/editor/<name>.rs` — `pub struct Name` + `impl Editor for Name`
- [ ] `src/editor/mod.rs` — `mod <name>;` declaration
- [ ] `src/editor/mod.rs` — add `&<name>::Name` to `EDITORS`

## Notes for VS Code / Cursor contributors

VS Code exposes editor state through its extension API rather than a local
database. A VS Code backend would likely:

1. Ship a lightweight VS Code extension that writes active editor state
   (file paths, byte-offset selections) to a known file or Unix socket.
2. Implement `query()` by reading that state file / connecting to the socket.
3. For narration, register a keybinding in the extension's `package.json`
   that shells out to `attend narrate toggle`.
