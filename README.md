# `attend` is all you need

Speak your thoughts while navigating code, and `attend` uses a local
transcription model to send your words to your coding agent, interleaved with
what you were manipulating on-screen as you spoke.

I've found that pair programming using `attend`'s voice narration is a rather
different experience from typing my thoughts to a coding agent. There's
something very specific about *saying what I mean out loud* that forces me to
slow down and consider more deeply.

I invite you to see if you feel the same way.

## What it looks like

Press a hotkey, say out loud, *"this function should take a Duration instead of
a raw u64"* while your cursor is on the relevant code, and press the hotkey
again.

Your coding agent receives your words interleaved with edits and selections in
your text editor ([Zed](https://zed.dev)), selections from within your browser
([Firefox](https://www.mozilla.org/firefox/) or
[Chrome](https://www.google.com/chrome/)), clipboard contents (text and images),
and shell commands ([Fish](https://fishshell.com/) or
[Zsh](https://www.zsh.org/)), and even selections from arbitrary other apps
(macOS only) — all in chronological order, so it can understand your words and
actions in context with one another while you're narrating.

See [example narration](docs/example-narration.md) for what the agent receives.

## Installation

You'll need [Rust](https://rust-lang.org/learn/get-started) to install `attend`:

```bash
cargo install --locked --git https://github.com/oxidecomputer/attend attend
```

### Claude Code plugin (recommended)

The easiest way to integrate with Claude Code is to install the `attend`
plugin:

```
/plugin marketplace add oxidecomputer/attend
/plugin install attend@attend
```

Then write the required permissions (plugins cannot set these):

```bash
attend install --agent claude
```

When the plugin is detected, this writes only the permission grants that the
plugin needs. Without the plugin, it performs a full manual installation of
hooks and skills.

### Other integrations

Install editor, browser, and shell integrations:

```bash
attend install
```

This detects which integrations are available on your system (editors,
browsers, shells) and prompts you to confirm each one. It also provides the
option of pre-downloading a local transcription model, so that it's ready on
first-run (it will be downloaded anyhow when you first narrate).

Then follow the [getting started guide](docs/getting-started.md) to start your
first narrated session with your coding agent.

## Documentation

- [**Getting started**](docs/getting-started.md) — your first narration session,
  step by step
- [**Setup guide**](docs/setup.md) — hotkeys, browser and shell integration,
  transcription model, configuration, troubleshooting, and uninstall
- [**Command reference**](docs/commands.md) — every `attend` subcommand
- [**How it works**](docs/how-it-works.md) — architecture, data flow, and the
  merge pipeline
- [**Extending attend**](docs/extending.md) — adding support for new editors,
  agents, shells, and browsers
- [**Development**](docs/development.md) — building, testing, project structure,
  and dev installation
