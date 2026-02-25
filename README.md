# `attend` is all you need

Let your coding agent hear your voice and see what you're doing, as if you were
screen-sharing on a voice call with a collaborator.

When you're "pair programming" with an AI coding agent in your terminal, there's
a gap: the agent can see your files, but it can't see what you're seeing. Instead,
you have to tell it! You end up copy-pasting code snippets, typing out line numbers,
or vaguely describing context the agent would already have if it could see what you
do and hear you chat about it.

Speak your thoughts while navigating code, and `attend` transcribes and delivers
them as prompts. You can highlight code, flip between files, and narrate what you
want done, without leaving your editor or switching to a chat window. The agent
receives your words interleaved with what you were looking at or editing as you
spoke.

Even when you're not actively narrating, `attend` queries your editor for changes
in visible files, cursor positions, and selections, then injects that context into
the conversation, so your coding agent knows what's in front of you.

Personally, I've found "pair programming" using `attend`'s voice narration is
a rather different experience from typing my thoughts to a coding agent. There's
something very specific about *saying what I mean out loud* that forces me to
slow down and consider more deeply.

I invite you to see if you feel the same way.

## Supported editors, agents, browsers, and platforms

- Editors: [Zed](https://zed.dev)
- Agents: [Claude Code](https://claude.com/product/claude-code)
- Browsers: [Firefox](https://www.mozilla.org/firefox/) (via native messaging extension)
- Platforms: anything Unix-esque should work; Windows is not supported currently

The architecture supports adding new editors, agents, and browsers independently
of one another. See [EXTENDING.md](EXTENDING.md) for how to implement new editor
or agent integrations. Contributions welcome!

## Platform requirements

`attend` requires a Unix platform (macOS or Linux). Windows is not currently
supported.

## Quick start

### Installation

To install `attend`, you'll need
[Rust](https://rust-lang.org/learn/get-started/):

```bash
cargo install --git https://github.com/oxidecomputer/attend
attend install --agent claude --editor zed
```

The final step installs the all-important hooks that provide editor context to
Claude Code, plus keybindings for narration control (toggle, start, pause,
yank) from within Zed. See [Narration hotkeys](#narration-hotkeys) for
the full list.

### Browser integration (optional)

To capture text selections from your browser and deliver them as narration
context:

```bash
attend install --browser firefox   # or: --browser chrome
```

For Firefox, this installs a native messaging host manifest and opens the
signed extension for installation. After clicking "Add" in Firefox, the
extension persists across restarts.

For Chrome, this installs a native messaging host manifest and writes an
unpacked extension to a persistent directory. You then load it manually:
open `chrome://extensions`, enable Developer mode, click "Load unpacked",
and select the directory printed by the install command.

When narration is active, text you select in the browser will be captured
with the page URL and title, and delivered to your agent alongside speech
and editor context.

### Shell integration (optional)

To capture shell commands (what you ran, exit status, duration) as narration
context:

```bash
attend install --shell fish   # or: --shell zsh
```

This installs hooks that fire on every command. When narration is active,
commands you run in that shell are captured and delivered alongside speech and
editor context, so the agent can see what you executed.

### Narration hotkeys

Narration is controlled by four commands. You'll have the best experience if
these are bound to hotkeys accessible without leaving your editor.

| Command | Purpose |
|---------|---------|
| `attend narrate toggle` | Start narration, or send and stop |
| `attend narrate start` | Start narration, or send and keep recording |
| `attend narrate pause` | Pause/resume recording |
| `attend narrate yank` | Stop and copy narration to clipboard |

#### Zed

`attend install --editor zed` installs keybindings and tasks automatically:

| macOS | Linux | Task |
|-------|-------|------|
| `⌘ ;` | `Super ;` | attend: toggle narration |
| `⌘ :` | `Super :` | attend: start narration |
| `⌘ {` | `Super {` | attend: pause narration |
| `⌘ }` | `Super }` | attend: yank narration |

Reinstallation respects any keybinding changes you've made in Zed.

#### Global hotkeys (macOS / Linux)

If you use a hotkey manager that can assign commands to keys, you can bind
*global* hotkeys to the narrate subcommands. On macOS, you can [bind a
global keyboard shortcut to a script using the Shortcuts
app](https://support.apple.com/guide/shortcuts-mac/launch-a-shortcut-from-another-app-apd163eb9f95/mac).
Your favorite Linux distribution almost certainly has some way to do this
too.

#### iTerm2

iTerm2 does not pick up macOS global hotkeys. To use the same keybindings
from iTerm2, add key mappings under Settings > Keys > Key Bindings:

1. Click **+** to add a new binding.
2. Set the shortcut, action **Run Coprocess**, and the corresponding command.
3. Ensure it is marked as "Apply to current session".
4. Repeat for each shortcut you want.

| Shortcut | Command |
|----------|---------|
| `⌘;` | `~/.cargo/bin/attend narrate toggle` |
| `⌘:` | `~/.cargo/bin/attend narrate start` |
| `⌘{` | `~/.cargo/bin/attend narrate pause` |
| `⌘}` | `~/.cargo/bin/attend narrate yank` |

Use the full path (`~/.cargo/bin/attend`) because iTerm2 coprocesses run
under `/bin/sh`, which does not have `~/.cargo/bin` in its PATH.

### Agent integration

For the agent to receive your narration, ask it to attend.

In Claude Code, this is done with the `/attend` slash-command. If you use multiple
Claude Code sessions, you can move narration from one session to another by invoking
`/attend` in whichever session you'd like to switch to.

Insofar as the agent doesn't ask for keyboard input (i.e. by presenting a plan,
asking a multiple choice question, or requesting permission to do an action),
you need never leave your focused editor, because you can narrate your responses
while you're in the codebase.

As a security precaution, the agent only sees editor context (cursors, selections,
file contents, diffs) from within its own working directory. If you navigate to
files elsewhere in your editor, the agent won't be able to follow along. You can
expand this with `include_dirs` in the config file
(see [Configuration](#configuration)).

### Troubleshooting

Run `attend narrate status` to check that everything is wired up correctly. It
shows whether narration is recording, which engine and session are active,
whether the editor integration is healthy, and whether any narration is pending.

#### Microphone permissions (macOS)

The recording daemon needs microphone access. macOS prompts for permission the
first time the daemon is launched from a given parent process. If narration
starts but produces no speech, check **System Settings > Privacy & Security >
Microphone** and ensure the **focused app** has microphone permissions,
because the keyboard shortcut is triggering the script from within that app's
context.

If the permission prompt never appeared, the daemon may have been blocked
silently. Try running `attend narrate toggle` directly in a terminal to trigger
the prompt.

### Transcription model

Narration uses a local speech-to-text model: no audio leaves your machine. By
default, the first time you start recording, the model is automatically downloaded
from [Hugging Face](https://huggingface.co/models) and cached locally.

Two engines are available:

| Engine | Default Model | Size | Notes |
|--------|---------------|------|-------|
| `parakeet` (default) | [Parakeet TDT 0.6B](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) | ~1.2 GB | Better quality, multilingual, faster |
| `whisper` | [Whisper Small (GGML)](https://huggingface.co/ggerganov/whisper.cpp) | ~466 MB | Smaller, English only, slower |

To change the engine, see [Configuration](#configuration).

## Configuration

`attend` loads config from two sources, merged together:

- **Project**: `.attend/config.toml` in the current directory or any parent
  (closer files take precedence for scalar values; arrays are concatenated)
- **Global**: `~/.config/attend/config.toml`

All fields are optional:

```toml
engine = "parakeet"                        # transcription engine: "parakeet" or "whisper"
model = "/path/to/custom/model"            # custom model path (auto-downloaded if omitted)
include_dirs = ["/path/to/other/project"]  # additional dirs visible to the agent
archive_retention = "7d"                   # auto-prune old narrations ("forever" to disable)
```

## Uninstall

To remove everything:

```bash
attend uninstall
cargo uninstall attend
```

Or, specify a particular `--agent` or `--editor` for `attend uninstall` to
uninstall only the integrations for that.

## Commands

### Standalone tools

These let you inspect your editor state directly from the terminal. Useful for
debugging, demos, and understanding what attend sees.

#### `attend glance`

Print the current editor state (visible files + positions):

```bash
$ attend glance
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection). Add `--watch` for a
live-updating view, or `--format json` for structured output.

#### `attend look`

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

#### `attend meditate`

Run as a background daemon that continuously updates the editor state cache
without producing output.

If you are not using narration, running this in the background mildly improves 
the accuracy of the editor context provided to your agent at every turn, because
it maintains a more precise ordering of which cursors or selections you most
recently touched. This is only relevant in the case of multiple editor panes,
selections, or cursors.

### Janitorial commands

#### `attend narrate status`

Show narration system status, including a report of any problems that are detected.

#### `attend narrate clean`

In case of problems in the agent harness, you don't want to lose your narration and
have to say it all over again! That's why `attend` maintains an archive of all your
narrations. By default, archives older than 7 days are automatically pruned after
each narration delivery (configurable via `archive_retention` in the config file).

You can also remove old archived narration files manually using this command, which
defaults to cleaning everything older than 7 days.

### Narration commands

See [Narration hotkeys](#narration-hotkeys) for the commands and how to
bind them. You can also run them directly in a terminal:

```bash
attend narrate toggle   # start, or send and stop
attend narrate start    # start, or send and keep recording
attend narrate stop     # send and stop (no toggle)
attend narrate pause    # pause/resume
attend narrate yank     # stop and copy to clipboard
```

### Agent integration

These are the commands that make the pair programming experience work. You
typically don't run them directly: your coding agent does.

| Command | Purpose |
|---------|---------|
| `attend hook --agent <agent> <event>` | Run a hook event (session-start, user-prompt, stop) |
| `attend listen` | Wait for narration and deliver it to the agent |
| `attend listen --check` | Check for pending narration without waiting |

## Development

```
cargo fmt
cargo clippy
cargo test
cargo build --release
```

Globally install `attend` hooks pointed at your local fork like this:

```bash
cargo run -- install --dev --agent <agent> --editor <editor>
```
