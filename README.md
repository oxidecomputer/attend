# `attend` is all you need

Let your coding agent hear your voice and see what you're doing, as if you were
screen-sharing on a voice call with a collaborator.

Speak your thoughts while navigating code, and `attend` transcribes and delivers
them as prompts: your words interleaved with what you were looking at as you
spoke. Even when you're not actively narrating, `attend` queries your editor for
cursor positions and selections, and injects that context into the conversation
so your coding agent knows what's in front of you.

## What ends up in the narration

A narration weaves together up to seven sources of context, interleaved
chronologically:

- **Voice**: your speech, transcribed to text via a local model. Always
  available when recording.
- **Editor snapshots**: the code around your cursor or selection, with file
  path and language. Captured whenever you navigate or select. Requires editor
  integration.
- **File diffs**: the net change to files you edited while speaking. Requires
  editor integration.
- **External selections**: text you highlighted in any application, captured
  via the accessibility API. macOS only; requires granting the accessibility
  permission.
- **Browser selections**: rich text you selected on a web page, with the page
  URL and title. Requires a browser extension.
- **Clipboard**: text *or images* you copied (Cmd+C) during recording. Text
  that duplicates a richer source (browser or external selection) is
  automatically dropped.
- **Shell commands**: commands you ran, with exit status and duration. Requires
  shell hook integration.

Editor snapshots, file diffs, and shell commands are scoped to the agent's
working directory; those from outside it are marked as redacted.

Personally, I've found "pair programming" using `attend`'s voice narration is
a rather different experience from typing my thoughts to a coding agent. There's
something very specific about *saying what I mean out loud* that forces me to
slow down and consider more deeply.

I invite you to see if you feel the same way.

## Supported editors, agents, browsers, shells, platforms...

- Editors: [Zed](https://zed.dev)
- Agents: [Claude Code](https://claude.com/product/claude-code)
- Browsers (optional): [Firefox](https://www.mozilla.org/firefox/),
  [Chrome](https://www.google.com/chrome/)
- Shells (optional): [Fish](https://fishshell.com/), [Zsh](https://www.zsh.org/)
- Platforms: anything Unix-esque should work (if it doesn't, it's a bug!);
  Windows is not supported currently

The architecture supports adding new editors, agents, shells, and browsers
independently of one another. See the [extending guide](docs/extending.md)
for how to add new integrations. Contributions welcome!

## Quick start

To install `attend`, you'll need
[Rust](https://rust-lang.org/learn/get-started/); then, you should:

```bash
cargo install --git https://github.com/oxidecomputer/attend
attend install --agent claude --editor zed    # for example
```

This installs the hooks that provide editor context to Claude Code, plus
keybindings for narration control (toggle, start, pause, yank) from within
Zed. See [Narration hotkeys](docs/setup.md#narration-hotkeys) for the full
list.

Optional integrations capture additional context while you narrate:

```bash
attend install --browser firefox   # or: --browser chrome
attend install --shell fish        # or: --shell zsh
```

See the [setup guide](docs/setup.md) for details on each integration.

## Next steps

- [**Setup guide**](docs/setup.md) — browser integration, shell integration,
  narration hotkeys, agent integration, transcription model, configuration,
  troubleshooting, and uninstall
- [**Command reference**](docs/commands.md) — standalone tools, narration
  commands, and agent integration commands
- [**Extending attend**](docs/extending.md) — how to add support for new
  editors and agents
- [**Development**](docs/development.md) — building, testing, dev
  installation, and xtasks
