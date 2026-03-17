# Your first narration session

This guide walks you through narrating for the first time. You'll check your
installation, activate narration in your agent, and speak your first narration.

Before starting, you need two things:

1. **Install `attend`** by following the [installation
   instructions](../../README.md#installation) in the README. This installs the
   binary and integrations.
2. **Start a Claude Code session** in a project directory. The tutorial assumes
   you have the `attend` Claude Code plugin installed (the recommended path).

If you run into trouble at any point, see
[troubleshooting](../how-to/troubleshooting.md).

## 1. Check your installation

Run `attend narrate status` to verify everything is wired up:

```
Recording:      idle
Engine:         Parakeet TDT (model downloaded)
Idle timeout:   5m (default)
Session:        none
Listener:       inactive
Editors:        zed (ok)
Shells:         fish (ok), zsh (ok)
Browsers:       chrome (ok), firefox (ok)
Accessibility:  ok
Clipboard:      enabled
Pending:        0 narration(s)
Archive:        0 B

Paths:
  Cache:      ~/Library/Caches/attend
  Archive:    ~/Library/Caches/attend/narration/archive
  Lock:       ~/Library/Caches/attend/daemon/lock
  Config:     ~/.config/attend/config.toml
```

The important lines:

- **Engine** should show a recognized engine. The model downloads
  automatically when you first run `/attend` in your agent, or when you run
  `attend narrate model download`.
- **Editors** should show your editor with `(ok)`.
- **Accessibility** should show `ok` on macOS. If it doesn't, see
  [troubleshooting](../how-to/troubleshooting.md#macos-permissions).

## 2. Activate narration for your agent

In your Claude Code session, type:

```
/attend:start
```

This tells `attend` to deliver narration to *this session* and nowhere else.
The agent is instructed not to emit confirmation, but you should see a
background task start `attend listen`.

(If you installed manually without the plugin, use `/attend` instead.)

Without this step, narration will have no place to be delivered, and will
default to the system clipboard instead.

**Switching sessions:** if you have multiple Claude Code sessions, run the
activation command in whichever one you want to receive narration. The previous
session releases ownership automatically.

## 3. Narrate

1. **Open a file** in your editor and place your cursor on something
   interesting — a function, a struct, a comment.

2. **Press the toggle hotkey** to start recording:
   - Zed on macOS: `⌘ ;`
   - Zed on Linux: `Super ;`
   - Or run `attend narrate toggle` in a terminal.

3. **Speak your thoughts** while looking at code. For example: *"I want to
   refactor this function to take a config struct instead of individual
   parameters."* Move your cursor or select code as you talk — `attend`
   captures where you're looking.

4. **Press the toggle hotkey again** to stop recording and deliver. The
   narration is transcribed, merged with the editor context you produced
   while speaking, and sent to your agent.

5. **Watch your agent respond.** It received your words interleaved with the
   code you were looking at — both what you said and the context around it.
   You should see the agent acknowledge your narration and start working on
   whatever you asked for, just as if you had typed it.

The first time you record, macOS will prompt you to grant Microphone access.
Grant it. See [troubleshooting](../how-to/troubleshooting.md#macos-permissions) if the
prompt doesn't appear.

## Next steps

Now that you've narrated for the first time, see the [hotkey
guide](../how-to/hotkeys.md) to set up global hotkeys so you can narrate from
any application. To understand the full picture of what happened behind the
scenes, see [how it works](../explanation/how-it-works.md).
