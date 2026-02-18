# `attend` to your editor

For when you just want to point at some code and say, "fix this!"

This tool reads your editor's current state (visible open files, cursor
positions, selections) and delivers it as context that a coding agent can use to
understand what you are looking at.

Currently supports Zed (editor) and Claude Code (agent). The architecture is
intended to support other editors and agents; see [EXTENDING.md](EXTENDING.md)
for how to add new backends.

## How it works

The tool queries the editor (using different strategies depending on the editor)
for selection and cursor locations in visible panes, resolves byte offsets to
line:column positions by scanning files on disk, and orders output by recency so
visible files with recently moved cursors or selections appear first, with their
cursors and selections ordered by recency.

A per-session cache suppresses output when nothing has changed, to preserve
context.

## Usage

```
attend [-d <PATH>] [-f human|json]
```

`--dir`/`-d` resolves paths relative to that directory and shows relative paths.
Available globally (applies to both the default output and `view`).

With no subcommand, prints the current editor state to stdout and exits.

### Output format

```
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection). `--format json` emits
a JSON object with a `files` array.

### `view` — show file content at positions

```
attend view [--full] [-B <N>] [-A <N>] <path> <pos>... [<path> <pos>...]
```

Reads files from disk and prints the content at the given cursor/selection
positions. Accepts the same compact `path line:col...` format that the default
output produces, so you can pipe one into the other.

```
attend view src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
```

Cursors are marked with `❘` and selections with `⟦⟧` (or ANSI inverse
video on a TTY). By default only the lines spanned by each position are shown.
`-B`/`-A` add context lines before/after, and `--full` shows the entire file
with highlights inline. Overlapping context ranges are merged into a single
group.

Input can also be piped on stdin with `-`:

```
attend | attend view -
```

## Agent integration

When installed as a hook, the agent receives an `<editor-context>` block
before each prompt — but only when the editor state has actually changed
since the last prompt. This keeps the agent aware of what the user is looking
at without repeating stale information.

Installing the hook in your agent also adds instructions teaching the agent how
to use `attend view` so it can more effectively see precisely what you've
selected (rather than "mentally" calculating column offsets).

### Install

```
attend hook install --agent claude
```

This writes two hook entries into the agent's settings file:

- **SessionStart** — clears the per-session cache and emits format
  instructions that teach the agent how to read `<editor-context>` blocks.
- **UserPromptSubmit** — queries the editor, compares against the cache, and
  emits an `<editor-context>` block only if something changed.

Add `--project <PATH>` installs to a project-local settings file instead of
global.

Using `--dev` embeds the absolute binary path rather than relying on `$PATH`, so
you can install the hooks globally but point them at a local development build
of this tool.

### Uninstall

```
attend hook uninstall --agent claude
```

Add `--project <PATH>` to target a project-local settings file.

## Building

```
cargo build --release
```

## Testing

```
cargo test
```
