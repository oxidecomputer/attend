# Adding a new editor

An editor backend reads open files from whatever source the editor exposes
(database, socket, file, CLI) and returns a `QueryResult`.

## 1. Create the module — `src/editor/<name>.rs`

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

## 2. Register the backend in `src/editor/mod.rs`

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

## Checklist

- [ ] `src/editor/<name>.rs` — `pub struct Name` + `impl Editor for Name`
- [ ] `src/editor/mod.rs` — `mod <name>;` declaration
- [ ] `src/editor/mod.rs` — add `&<name>::Name` to `EDITORS`

## Notes for future VS Code / Cursor contributors

VS Code exposes editor state through its extension API rather than a local
database. A VS Code backend would likely:

1. Ship a lightweight VS Code extension that writes active editor state
   (file paths, byte-offset selections) to a known file or Unix socket.
2. Implement `query()` by reading that state file / connecting to the socket.
3. For narration, register a keybinding in the extension's `package.json`
   that shells out to `attend narrate toggle`.
