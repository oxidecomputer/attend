# `attend` to your editor

Connects editors to AI coding agents, regardless of whether they're natively
integrated. Reads the editor's current state (visible open files, cursor
positions, selections, terminal working directories) and delivers it as context
that the agent can use to understand what the user is looking at.

Currently supports Zed (editor) and Claude Code (agent). The architecture is
intended to support other editors and agents; contributions are welcome.

## How it works

The tool queries the editor (using different strategies depending on the editor)
for selection and cursor locations in visible panes, resolves byte offsets to
line:column positions by scanning files on disk, and orders output by recency so
visible files with recently moved cursors or selections appear first, with their
cursors and selections ordered by recency. A per-session cache suppresses output
when nothing has changed.

## Usage

```
attend [-d <PATH>] [-f human|json]
```

With no subcommand, prints the current editor state to stdout and exits.
`--dir`/`-d` filters to files under that directory and makes paths relative.

### Output format

```
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
~/project $
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection). Lines ending with `$`
are terminal working directories. `--format json` emits a JSON object with
`files` and `terminals` arrays.

## Agent integration

When installed as a hook, the agent receives an `<editor-context>` block
before each prompt — but only when the editor state has actually changed
since the last prompt. This keeps the agent aware of what the user is looking
at without repeating stale information.

### Install

```
attend hook install --agent claude
```

This writes two hook entries into the agent's settings file:

- **SessionStart** — clears the per-session cache and emits format
  instructions that teach the agent how to read `<editor-context>` blocks.
- **UserPromptSubmit** — queries the editor, compares against the cache, and
  emits an `<editor-context>` block only if something changed.

`--project <PATH>` installs to a project-local settings file instead of
global. `--dev` embeds the absolute binary path rather than relying on
`$PATH`.

### Uninstall

```
attend hook uninstall --agent claude
```

`--project <PATH>` to target a project-local settings file.

## Building

```
cargo build --release
```

Requires Rust 2024 edition (1.85+). The SQLite driver is bundled via
`rusqlite`; no system library needed.

## Testing

```
cargo test
```

The test suite includes property-based tests (proptest) for offset resolution
and reorder invariants, plus integration tests that simulate multi-invocation
hook sessions with real files on disk.
