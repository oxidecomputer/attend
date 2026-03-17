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

See [example narration](docs/tutorial/example-narration.md) for what the agent receives.

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

Then follow [your first narration session](docs/tutorial/getting-started.md) to
start narrating with your coding agent.

## Documentation

**Start here:**
- [**Your first narration session**](docs/tutorial/getting-started.md) — a
  step-by-step tutorial for your first narration
- [**Example narration**](docs/tutorial/example-narration.md) — see what the
  agent actually receives

**How-to guides:**
- [**Narrating effectively**](docs/how-to/narrating-effectively.md) — get the
  most out of voice-driven pair programming
- [**Hotkeys**](docs/how-to/hotkeys.md) — configure narration hotkeys in Zed,
  globally, and in iTerm2
- [**Browser integration**](docs/how-to/browser-integration.md) — capture text
  selections from Firefox and Chrome
- [**Shell integration**](docs/how-to/shell-integration.md) — capture shell
  commands from Fish and Zsh
- [**Troubleshooting**](docs/how-to/troubleshooting.md) — diagnose common
  problems and macOS permissions
- [**Uninstall**](docs/how-to/uninstall.md) — remove attend and its
  integrations

**Reference:**
- [**Command reference**](docs/reference/commands.md) — every `attend`
  subcommand
- [**Configuration**](docs/reference/configuration.md) — config files, fields,
  and transcription engines
- [**Narration format**](docs/reference/narration-format.md) — event types and
  rendering format
- [**Extending reference**](docs/extending/reference.md) — trait APIs, hook
  events, and message templates

**Understanding attend:**
- [**How it works**](docs/explanation/how-it-works.md) — data flow, the
  recording daemon, sessions, and the merge pipeline
- [**Architecture**](docs/explanation/architecture.md) — design patterns, test
  architecture, and rationale

**Extending and contributing:**
- [**Extending attend**](docs/extending/) — adding support for new editors,
  agents, shells, and browsers
- [**Development**](docs/extending/development.md) — building, testing, and dev
  installation
