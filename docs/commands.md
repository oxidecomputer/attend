# Commands

## Standalone tools

These let you inspect your editor state directly from the terminal. Useful for
debugging, demos, and understanding what attend sees.

### `attend glance`

Print the current editor state (visible files + positions):

```bash
$ attend glance
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection). Add `--watch` for a
live-updating view, or `--format json` for structured output.

### `attend look`

Reads files from disk and prints content with cursors and selections overlaid.

When writing to a TTY, cursors and selections are marked with inverse-video;
otherwise, or when `NO_COLOR` is set, cursors are marked with `❘` and selections
with `⟦⟧`.

Show file content with specific cursors/selections applied:

```bash
$ attend look src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
```

Cursors/selections are given using the same format output by `attend glance`,
and `attend look -` parses from stdin:

```
attend glance | attend look -
```

You can also combine these into the equivalent shortcut:

```bash
attend look
```

Use `attend look --watch` to get a live-updated view in your terminal (or a
newline-separated stream of updates, if stdout is not a TTY).

Use `-B`/`-A` for additional context lines, or `--full` for the entire file.

Use `--format json` for machine-readable output.

**Caveat**: Because `attend look` reads the live editor selection state, but
shows file contents from disk, results from unsaved files may not be accurate.

### `attend meditate`

Run as a background daemon that continuously updates the editor state cache
without producing output.

If you are not using narration, running this in the background mildly improves
the accuracy of the editor context provided to your agent at every turn, because
it maintains a more precise ordering of which cursors or selections you most
recently touched. This is only relevant in the case of multiple editor panes,
selections, or cursors.

## Janitorial commands

### `attend narrate status`

Show narration system status, including a report of any problems that are detected.

### `attend narrate clean`

In case of problems in the agent harness, you don't want to lose your narration and
have to say it all over again! That's why `attend` maintains an archive of all your
narrations. By default, archives older than 7 days are automatically pruned after
each narration delivery (configurable via `archive_retention` in the config file).

You can also remove old archived narration files manually using this command, which
defaults to cleaning everything older than 7 days.

## Narration commands

See [Narration hotkeys](setup.md#narration-hotkeys) for the commands and how to
bind them. You can also run them directly in a terminal:

```bash
attend narrate toggle   # start if idle, or send and stop if recording
attend narrate start    # start if idle, or send and keep recording
attend narrate stop     # send and stop (no toggle)
attend narrate pause    # pause/resume
attend narrate yank     # stop, exit daemon, and copy to clipboard
```

The recording daemon is **persistent**: when you stop recording, the daemon
flushes content and enters an idle state with the transcription model still
loaded. The next `toggle` or `start` resumes instantly without reloading the
model or spawning a new process. The daemon auto-exits after an idle timeout
(default 5 minutes; configurable via `daemon_idle_timeout`).

## Agent integration

These are the commands that make the pair programming experience work. You
typically don't run them directly: your coding agent does.

| Command | Purpose |
|---------|---------|
| `attend hook --agent <agent> <event>` | Run a hook event (session-start, user-prompt, stop) |
| `attend listen` | Wait for narration and deliver it to the agent |
| `attend listen --check` | Check for pending narration without waiting |
| `attend listen --stop` | Deactivate narration: remove the listening file and exit |
