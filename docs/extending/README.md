# Extending attend

Each integration type has its own guide:

- [**Agents**](agents.md) — hook into an AI agent's lifecycle to deliver editor
  context and narration
- [**Editors**](editors.md) — read editor state (open files, cursors,
  selections) and provide narration hotkeys
- [**Shells**](shells.md) — capture shell commands as narration context
- [**Browsers**](browsers.md) — capture browser text selections as narration
  context

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

For details on each trait's API, see the [extending
reference](reference.md). For a higher-level overview of how the system fits
together, see [how it works](../explanation/how-it-works.md) and
[architecture](../explanation/architecture.md).
