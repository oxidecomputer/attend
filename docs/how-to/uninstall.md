# How to uninstall `attend`

Remove integrations first, then the plugin (if installed), then the binary.

## Remove all integrations

```bash
attend uninstall
```

This removes agent hooks, editor keybindings, browser extensions, and shell
hooks — including all tracked project-local installations.

## Or, remove only specific integrations

```bash
attend uninstall --agent claude
attend uninstall --editor zed
attend uninstall --browser firefox
attend uninstall --shell fish
```

## Remove the plugin

If you installed the Claude Code plugin:

```
/plugin uninstall attend@attend
/plugin marketplace remove attend
```

## Remove the binary

```bash
cargo uninstall attend
```
