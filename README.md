# `zc`: Zed Context

Reads the current state of the [Zed](https://zed.dev) editor — visible files,
cursor positions, selections, and terminal working directories — and outputs it
for consumption by humans or (primarily) AI coding agents.

It works by querying Zed's internal SQLite database (read-only), resolving byte
offsets to line:column positions by scanning the actual files on disk, and
optionally ordering output by recency so the most recently touched files and
cursors appear first.

## Usage

```
zc [-d <PATH>] [-f human|json]
```

With no subcommand, prints the current editor state to stdout and exits.

`--dir`/`-d` filters to files under the specified directory and makes paths
relative to it.

### Output format

```
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
~/project $
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection). Lines ending with `$`
are terminal working directories. With `--format json`, output is a JSON object
with `files` and `terminals` arrays.

## Agent integration

Currently supports [Claude Code](https://claude.com/product/claude-code).
Expansion to other agents is planned, PRs are welcome.

Once installed as a hook in the agent, the agent receives a `<zed-context>`
block only if the user has changed their visible editor state since the last
time the hook was run. This provides an up-to-date reference to what the user is
looking at, so the agent can more easily understand the user's work in context.

### Install hooks

```
zc hook install --agent claude
```

This writes two entries into the agent's settings file:

- **SessionStart** — clears the per-session cache and emits format instructions
  that teach the agent how to read `<zed-context>` blocks.
- **UserPromptSubmit** — queries Zed, compares against cached state, and emits a
  `<zed-context>` block only if something changed.

Use `--project <PATH>` to install into a project-local `.claude/settings.json`
instead. Use `--dev` to embed the absolute binary path rather than relying on
`$PATH`.

### Uninstall

```
zc hook uninstall --agent claude
```

Use `--project <PATH>` to install into a project-local `.claude/settings.json`
instead.

### How the cache works

Each session gets a JSON cache file under the platform cache directory (e.g.
`~/.cache/zed-context/` on Linux). On each prompt the hook builds the current
state, reorders it by recency against the cached previous state, and compares.
If nothing changed the hook is silent. Otherwise it writes the new state to the
cache and emits output. The cache is cleared on session start.

The effect of this is that the hook only runs when the user has changed their
visible file(s), cursor position(s), selection(s), or terminal(s) since the last
time such information was reported to the agent, so the agent is always up to
date about what you're looking at.

## Building

```
cargo build --release
```

Requires Rust 2024 edition (1.85+). The SQLite driver is bundled via `rusqlite`
so no system library is needed.

## Testing

```
cargo test
```

The test suite includes property-based tests for offset resolution and reorder
invariants, plus integration tests that simulate multi-invocation hook sessions
with real files on disk.

## How it finds the database

Zed stores its state in SQLite databases under the platform data directory (via
the `dirs` crate — e.g. `~/Library/Application Support/Zed/db/` on macOS,
`~/.local/share/Zed/db/` on Linux). The tool picks the database whose
write-ahead log was most recently modified, opens it read-only, and queries the
`items`, `editors`, `editor_selections`, and `terminals` tables for active tabs.
