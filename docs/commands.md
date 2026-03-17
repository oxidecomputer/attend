# Command reference

## Narration commands

These control the recording lifecycle. You'll typically bind them to hotkeys
(see [Narration hotkeys](setup.md#narration-hotkeys)) rather than running them
in a terminal.

### `attend narrate toggle`

Start recording if idle. If already recording, stop, transcribe, and deliver the
narration to the active agent session.

### `attend narrate start`

Start recording if idle. If already recording, deliver the current narration and
**keep recording** — useful for continuous narration across multiple deliveries
without stopping the daemon.

### `attend narrate stop`

Stop recording, transcribe, and deliver. Unlike `toggle`, this is a no-op if
not currently recording (it won't start a new recording).

### `attend narrate pause`

Pause recording without delivering. Press again to resume. Audio capture and
context polling are suspended while paused.

### `attend narrate yank`

Stop recording, transcribe, and **copy the rendered narration to the clipboard**
instead of delivering it to an agent. Useful for pasting narration into other
contexts, or for seeing what `attend` produces.

## Inspection tools

These let you see what `attend` sees in your editor, directly from the terminal.
Useful for debugging, demos, and understanding the editor integration.

### `attend glance`

Print the current editor state — open files with cursor and selection positions:

```bash
$ attend glance
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection).

**Flags:**

| Flag | Description |
|------|-------------|
| `--watch`, `-w` | Live-updating view |
| `--dir`, `-d` | Resolve and display paths relative to this directory |
| `--format`, `-f` | Output format: `human` (default) or `json` |
| `--interval`, `-i` | Override polling interval in seconds |

### `attend look`

Read files from disk and print content with cursors and selections overlaid.

On a TTY, cursors and selections are highlighted with inverse video. Otherwise
(or when `NO_COLOR` is set), cursors are marked with `❘` and selections with
`⟦⟧`.

Show file content with specific positions:

```bash
$ attend look src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
```

Positions use the same format as `attend glance` output, and `attend look -`
reads positions from stdin:

```bash
attend glance | attend look -
```

With no arguments, `attend look` queries the editor and shows current state
(equivalent to the pipe above):

```bash
attend look
```

**Flags:**

| Flag | Description |
|------|-------------|
| `--watch`, `-w` | Live-updating view |
| `--full` | Show entire files (conflicts with `-B`/`-A`) |
| `-B` | Context lines before each excerpt |
| `-A` | Context lines after each excerpt |
| `--dir`, `-d` | Resolve and display paths relative to this directory |
| `--format`, `-f` | Output format: `human` (default) or `json` |
| `--interval`, `-i` | Override polling interval in seconds |

**Caveat:** `attend look` reads live editor selection state but shows file
contents from disk. Results from unsaved files may not be accurate.

### `attend meditate`

Run as a background daemon that continuously polls the editor and warms the
state cache, without producing output.

If you're not using narration, running this in the background mildly improves
the accuracy of editor context provided to your agent at every turn, because it
maintains a more precise ordering of which cursors or selections you most
recently touched. Only relevant with multiple editor panes or cursors.

**Flags:**

| Flag | Description |
|------|-------------|
| `--interval`, `-i` | Override polling interval in seconds |

## Setup commands

### `attend install`

Install agent hooks, editor keybindings, browser extensions, and shell hooks.

```bash
attend install --agent claude --editor zed
attend install --browser firefox --shell fish
```

With no flags, `attend install` detects which integrations are available on your
system and prompts you to confirm each one. With explicit flags, it installs
only what you specify and fails on errors.

If the `attend` Claude Code plugin is already enabled, `--agent claude` writes
only the permission grants that the plugin needs (plugins cannot set
permissions). Without the plugin, it performs a full manual installation of
hooks and skills.

**Flags:**

| Flag | Description |
|------|-------------|
| `--agent`, `-a` | Agent to install hooks for (e.g., `claude`). Repeatable. |
| `--editor`, `-e` | Editor to install keybindings for (e.g., `zed`). Repeatable. |
| `--browser`, `-b` | Browser to install extension for (e.g., `firefox`, `chrome`). Repeatable. |
| `--shell`, `-s` | Shell to install hooks and completions for (e.g., `fish`, `zsh`). Repeatable. |
| `--project`, `-p` | Install to project-local settings instead of global |
| `--dev` | Point hooks at the current binary path (for development) |

### `attend uninstall`

Remove installed integrations. With no flags, removes everything (including
all tracked project-local installations). With explicit flags, removes only the
specified integrations.

**Flags:**

| Flag | Description |
|------|-------------|
| `--agent`, `-a` | Agent to uninstall hooks for. Repeatable. |
| `--editor` | Editor to uninstall keybindings for. Repeatable. |
| `--browser`, `-b` | Browser to uninstall extension for. Repeatable. |
| `--shell`, `-s` | Shell to uninstall hooks for. Repeatable. |
| `--project`, `-p` | Remove from a project-local settings file instead of global |

### `attend completions`

Generate shell completions and print to stdout:

```bash
attend completions fish > ~/.config/fish/completions/attend.fish
```

**Argument:** shell name (`bash`, `fish`, `zsh`, `elvish`, `powershell`).

Note: `attend install --shell <shell>` installs completions automatically;
this command is for manual setup.

## Maintenance commands

### `attend narrate status`

Show narration system status: recording state, engine, session, listener,
installed integrations, permissions, and any detected problems.

```
Recording:      idle
Engine:         Parakeet TDT (model downloaded)
Idle timeout:   5m (default)
Session:        a33c5803-8369-430d-9acf-70f24a5ba2d4
Listener:       active
Editors:        zed (ok)
Shells:         fish (ok)
Browsers:       firefox (ok)
Accessibility:  ok
Clipboard:      enabled
Pending:        0 narration(s)
Archive:        424.0 KB
```

This is the first thing to check when something isn't working.

### `attend narrate clean`

Remove old archived narrations. `attend` keeps delivered narrations as a safety
net (in case of agent crashes or delivery failures); they're pruned
automatically after each delivery based on `archive_retention` (default 7 days).

This command lets you clean up manually:

```bash
attend narrate clean                  # remove archives older than 7 days
attend narrate clean --older-than 1d  # remove archives older than 1 day
```

## Agent integration commands

These commands are called by your agent, not by you directly. They're documented
here for completeness.

### `attend hook`

Respond to agent lifecycle events. Called by the hooks that `attend install`
sets up.

```bash
attend hook --agent claude session-start
attend hook --agent claude user-prompt
attend hook --agent claude pre-tool-use
attend hook --agent claude post-tool-use
attend hook --agent claude stop
attend hook --agent claude session-end
```

### `attend listen`

Wait for pending narration and deliver it to the agent. Started as a background
task by the agent after `/attend` (or `/attend:start` with the plugin) is
invoked.

| Flag | Description |
|------|-------------|
| `--check` | Check once for pending narration and exit (no waiting) |
| `--stop` | Deactivate narration: remove the listening file and exit |
| `--session` | Session ID (defaults to the current listening session) |

## Internal commands

These are implementation details, not user-facing:

- `attend _record-daemon` — the persistent recording daemon (spawned by
  narration commands)
- `attend browser-bridge` — native messaging host for browser extensions
- `attend shell-hook preexec|postexec` — staging hook for shell command capture
