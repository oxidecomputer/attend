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
of one another. See [docs/extending.md](docs/extending.md) for how to implement
new editor or agent integrations. Contributions welcome!

## Platform requirements

`attend` requires a Unix platform (macOS or Linux). Windows is not currently
supported.

## Quick start

To install `attend`, you'll need
[Rust](https://rust-lang.org/learn/get-started/):

```bash
cargo install --git https://github.com/oxidecomputer/attend
attend install --agent claude --editor zed
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
