# Getting started

You've installed `attend` and run `attend install`. This guide walks you
through your first narration session.

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
  [Permissions](setup.md#permissions-macos).

## 2. Activate narration for your agent

In your agent session, run the activation slash command:

- **Plugin install:** `/attend:start` (stop with `/attend:stop`)
- **Manual install:** `/attend` (stop with `/unattend`)

This tells `attend` to deliver narration to *this session* and nowhere else.
The agent is instructed not to emit confirmation, but you should see a
background task start `attend listen`.

Without this step, narration will have no place to be delivered, and will
default to the system clipboard instead.

**Switching sessions:** if you have multiple Claude Code sessions, run the
activation command in whichever one you want to receive narration. The previous
session releases ownership automatically.

## 3. Your first narration

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
   code you were looking at: both what you said and the context around it.

The first time you record, macOS will prompt you to grant Microphone access.
Grant it. See [Permissions](setup.md#permissions-macos) if the prompt
doesn't appear.

## 4. What just happened

Behind the scenes:

- The **recording daemon** started (or resumed), captured audio from your
  microphone, and polled your editor for cursor positions every few hundred
  milliseconds.
- When you stopped, the audio was **transcribed locally** by a
  speech-to-text model running on your machine. No audio left your
  computer.
- Your transcribed words were **merged chronologically** with the editor
  snapshots, producing a markdown document where prose and code blocks
  alternate chronologically.
- The narration was **delivered** to your agent, which treated it as a
  prompt.

For the full picture, see [How it works](how-it-works.md).

## 5. Other narration commands

The toggle hotkey starts *and* stops narration. There are three other
commands for different workflows:

| Command | Hotkey (Zed, macOS) | What it does |
|---------|---------------------|--------------|
| `attend narrate start` | `⌘ :` | Start recording, or deliver current narration and keep recording |
| `attend narrate pause` | `⌘ {` | Pause/resume without delivering |
| `attend narrate yank` | `⌘ }` | Stop and copy narration to clipboard instead of delivering |

`start` is useful when you want to narrate continuously across multiple
deliveries without stopping the daemon. `yank` is useful for pasting
narration into other contexts (or just seeing what `attend` produces).

## 6. Next steps

- **Add browser integration** to capture text you select on web pages:
  `attend install --browser firefox` (see [Browser
  integration](setup.md#browser-integration))
- **Add shell integration** to capture commands you run:
  `attend install --shell fish` (see [Shell
  integration](setup.md#shell-integration))
- **Set up global hotkeys** so you can start/stop narration from any app,
  not just your editor (see [Global hotkeys](setup.md#global-hotkeys))
- **Customize configuration** — transcription engine, idle timeout, and
  more (see [Configuration](setup.md#configuration))
