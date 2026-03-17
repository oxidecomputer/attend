# How to add shell integration

This guide shows you how to capture shell commands (what you ran, exit status,
duration) as narration context.

## Fish

```bash
attend install --shell fish
```

The hook is installed to `~/.config/fish/conf.d/` and loaded automatically.

## Zsh

```bash
attend install --shell zsh
```

The hook is installed to `~/.config/attend/hooks/attend.zsh`. The installer
automatically adds the following line to your `~/.zshrc`:

```zsh
[[ -f ~/.config/attend/hooks/attend.zsh ]] && source ~/.config/attend/hooks/attend.zsh  # attend:hooks
```

## What you get

When narration is active, commands you run in that shell are captured and
delivered alongside speech and editor context. The agent sees what you executed,
the exit status, and how long it took. See [narration
format](../reference/narration-format.md#shell-commands) for how shell commands appear in
narration.
