# Extending attend

This document explains how to add support for a new editor, agent, shell, or
browser. Each integration type has its own guide:

- [**Editors**](extending-editors.md) — read editor state (open files, cursors,
  selections) and provide narration hotkeys
- [**Agents**](extending-agents.md) — hook into an AI agent's lifecycle to
  deliver editor context and narration
- [**Shells**](extending-shells.md) — capture shell commands as narration
  context
- [**Browsers**](extending-browsers.md) — capture browser text selections as
  narration context

## Architecture overview

```
editor/            Reads state from editor backends (Zed, etc.)
  mod.rs           Editor trait, merges results from all backends into QueryResult
  zed/             Zed backend (submodule directory)
    mod.rs         Query (SQLite), narration install, health checks
    ...

agent/             Hook installation and output rendering for each agent
  mod.rs           Agent trait, backend registry, resolve_bin_cmd
  messages/        Shared message templates (protocol descriptions, guidance)
  claude/          Claude Code agent backend
    mod.rs         Agent trait impl (delegates to submodules)
    ...

shell/             Shell hook installation for narration context capture
  mod.rs           Shell trait, backend registry
  fish.rs          Fish backend (conf.d hook + completions)
  zsh.rs           Zsh backend (preexec/precmd hooks + completions)

browser/           Browser extension installation for selection capture
  mod.rs           Browser trait, backend registry
  firefox.rs       Firefox backend (signed XPI + native messaging)
  chrome.rs        Chrome backend (unpacked extension + native messaging)
```

All four integration types follow the same pattern: implement a trait on a
zero-sized struct, register it in a static slice, and the CLI wires everything
up automatically.

## Supporting infrastructure

### Auto-upgrade

On each `SessionStart` hook, `attend` checks whether the running binary version
matches the version that installed the hooks (`~/.cache/attend/version.json`).
On mismatch, it automatically reinstalls all previously registered agents and
editors. This ensures hooks stay compatible after `cargo install` updates.

### Project path tracking

`attend install --project /path/to/project` records the path in
`InstallMeta.project_paths`. On `attend uninstall` (without `--project`),
all tracked project paths are cleaned up. This prevents stale
project-local config from accumulating.

### Narration delivery

Narration reaches the agent through two paths:

1. **Hook delivery** (non-blocking): The Stop, PreToolUse, and PostToolUse
   hooks collect pending narration files, render them as markdown wrapped in
   `<narration>` tags, and deliver via `attend_result(PendingNarration)`.
   PreToolUse and PostToolUse ensure narration arrives between tools within
   a single response, not just at the end.

2. **Background receiver** (blocking): When `attend_result(StartReceiver)`
   fires, the agent starts `attend listen` in the background. The receiver
   polls for pending files and prints them when they arrive, then exits so
   the agent can restart it for the next narration. This means that arriving
   narration can **prompt** a new conversational turn for the agent; without
   this mechanism, only narration *during* the agent's turn would register
   without manual intervention.

Both paths filter narration context to the project scope (cwd + `include_dirs`)
and relativize paths before delivery, so that there is no leak of file contents
from outside the agent's permissioned path.

### Receiver output protocol

The `attend listen` receiver is agent-agnostic. It uses a standard output
protocol based on XML tags:

- Narration content is wrapped in `<narration>` tags
- Operational instructions (restart, conflict) are wrapped in
  `<system-instruction>` tags

Each agent's instructions teach its LLM to expect this format. If an agent's
LLM requires fundamentally different framing, it can implement a custom
listener, but the default protocol works well for LLMs that handle XML tags.
