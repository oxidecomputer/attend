# attend

Attention is all you need, after all.

When you're working with an AI coding agent in your terminal, there's a gap: the
agent can see your files, but it can't see what *you're* doing — which file
you're reading, where your cursor is, what you've selected. You end up
copy-pasting code snippets, describing line numbers, or typing out context the
agent should already have. You can stop doing all that with `attend`.

## The experience

Once installed, `attend` runs as a hook inside your coding agent. Before each
prompt, it queries your editor for visible files, cursor positions, and
selections, then injects that context into the conversation — but only when
something has changed, to avoid repetition.

On top of that, attend supports **voice narration**: speak your thoughts while
navigating code, and attend transcribes and delivers them as prompts. You can
highlight code, flip between files, and narrate what you want done — all without
leaving your editor or switching to a chat window. The agent receives your words
interleaved with what you were looking at and what you changed, giving it the
full picture.

Personally, I've found "pair programming" using `attend` is a rather different
experience from typing my thoughts to a coding agent. There's something very
specific about *saying what you mean out loud* that forces me to slow down and
consider more deeply. I invite you to see if you feel the same way.

## Supported editors and agents

- Editors: [Zed](https://zed.dev)
- Agents: [Claude Code](https://claude.com/product/claude-code)

The architecture supports adding new editors and agents independently of one
another. See [EXTENDING.md](EXTENDING.md) for how to implement new editor or
agent integrations. Contributions welcome!

## Quick start

### Installation

To install `attend` from source, you'll need
[Rust](https://rust-lang.org/learn/get-started/):

```bash
git clone https://github.com/oxidecomputer/attend
cd attend
cargo install --path .
attend install --agent claude --editor zed
```

The final step installs hooks that automatically provide editor context to
Claude Code, plus keybindings for voice narration in Zed. Use `--dev` to point
hooks at a local development build, or `--project <PATH>` for project-local
settings.

### Editor hotkeys

By default, this installs two keybindings in Zed (if those aren't already
bound), as well as two named tasks they trigger:

- `⌘ :` starts narration. Pressing it again sends narration to the agent and
  keeps recording.
- `⌘ ;` toggles narration. Pressing it again sends narration to the agent and
  stops recording.

You can change these after the fact within Zed, and future reinstallation of Zed
hooks from `attend` will respect your preferences. To manually assign key
bindings, bind the tasks "attend: toggle narration" and "attend: flush
narration".

### Agent commands

In order for the agent to receive narration, you need to manually start it
listening. In Claude Code, this is done with the `/attend` slash-command. If you
use multiple Claude Code sessions, you can move narration from one session to
another by invoking `/attend` in whichever session you'd like to switch to.

Insofar as the agent doesn't ask for keyboard input (i.e. by presenting a plan,
asking a multiple choice question, or requesting permission to do an action),
you need never leave your focused editor, because you can narrate your responses
while you're in the codebase.

## Uninstall

To remove everything:

```bash
attend uninstall
```

Or, specify a particular `--agent` or `--editor`.

## Commands

### Agent integration

These are the commands that make the pair programming experience work. You
typically don't run them directly — they're invoked by hooks or keybindings.

| Command | Purpose |
|---------|---------|
| `attend narrate start` | Start the recorder |
| `attend narrate stop` | Stop the recorder |
| `attend narrate toggle` | Start or stop narration |
| `attend narrate flush` | Submit current narration and keep recording |
| `attend narrate status` | Show recording and system status |
| `attend narrate clean` | Remove old archived narration files |
| `attend hook --agent claude <event>` | Run a hook event (session-start, user-prompt, stop) |
| `attend listen` | Wait for narration and deliver it to the agent |
| `attend listen --check` | Check for pending narration without waiting |

### Standalone tools

These let you inspect your editor state directly from the terminal. Useful for
debugging, demos, and understanding what attend sees.

**`attend glance`**: print the current editor state (paths + positions):

```bash
$ attend glance
src/main.rs 14:3, 20:1-20:18
src/db.rs 1:1
```

Each line is a file path followed by comma-separated positions. A position is
`line:col` (cursor) or `line:col-line:col` (selection). Add `--watch` for a
live-updating view, or `--format json` for structured output.

**`attend look`**: show file content at cursor/selection positions:

```bash
$ attend look src/foo.rs 5:12 19:40-24:6 src/bar.rs 10:1
```

Reads files from disk and prints content at the given positions. When writing to
a TTY, cursors and selections are marked with inverse-video; when writing to a
pipe or when `NO_COLOR` is set, cursors are marked with `❘` and selections with
`⟦⟧`.

Use `-B`/`-A` for additional context lines, `--full` for the entire file, and
add `--watch` for live updates.

Input can also come from stdin with `-`:

```
attend glance | attend look -
```

**`attend meditate`**: run as a background daemon that continuously updates the
editor state cache without producing output. If you are not using narration,
running this in the background mildly improves the accuracy of the editor
context provided to your agent at every turn, because it keeps a more precise
tally of which cursor or selection was most recently used by you.

## Development

```
cargo fmt
cargo clippy
cargo test
cargo build --release
```
