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

Before starting, set up a [development environment](development.md) so you can
build, test, and dev-install your changes.

## The pattern

All four integration types follow the same pattern:

1. Implement a trait on a zero-sized struct.
2. Register it in a static slice in the parent module.
3. The CLI wires everything up automatically — `install`, `uninstall`, `hook`,
   and `narrate` all discover backends from the registry.

```
editor.rs  (trait)      agent.rs  (trait)       shell.rs  (trait)     browser.rs  (trait)
editor/                 agent/                  shell/                browser/
  zed/    (backend)       claude/ (backend)       fish.rs (backend)     firefox.rs (backend)
                                                  zsh.rs  (backend)     chrome.rs  (backend)
```

## Architecture context

For an overview of how the entire system fits together — data flow, the
recording daemon, session model, hook lifecycle — see
[How it works](how-it-works.md).

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
   hooks interrupt the agent whenever pending narration is available, preventing
   it from ending its turn or invoking any further tools until it receives the
   narration by re-invoking `attend listen` (whose PreToolUse hook actually
   delivers the narration). This ensures that the user can interrupt the agent
   at any time, even mid-turn, and that the agent must respond to narration
   before finishing its turn.

2. **Background receiver** (blocking): When `attend_result(StartReceiver)`
   fires, the agent starts `attend listen` in the background. The receiver polls
   for pending files and then exits immediately when they arrive so the agent
   can restart it for the next narration. Narration is delivered to the agent
   *exclusively* by the PreToolUse hook on `Bash(attend listen)`, forcing the
   agent to start the listener again in order to receive narration. This
   mechanism means that arriving narration can **prompt** a new conversational
   turn for the agent; without this mechanism, only narration *during* the
   agent's turn would register without manual intervention.

Delivered narration is filtered to the project scope (cwd + `include_dirs`) and
paths are relativized before delivery, so that there is no leak of file contents
from outside the agent's permissioned path. (This does not apply to
non-project-scoped capture sources such as the system clipboard, accessibility
API, or browser extension).

### Receiver output protocol

The `attend listen` receiver is agent-agnostic. It uses a standard output
protocol based on XML tags:

- Narration content is wrapped in `<narration>` tags
- Operational instructions (restart, conflict) are wrapped in
  `<system-instruction>` tags

Each agent's instructions teach its LLM to expect this format. If an agent's
LLM requires fundamentally different framing, it can implement a custom
listener.
