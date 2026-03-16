# Setup guide

This guide covers optional integrations, configuration, and troubleshooting. For
initial installation, see the [quickstart in the
README](../README.md#quick-start).

## Agent integration

For the agent to receive your narration, ask it to attend.

In Claude Code, this is done with the `/attend` slash-command. If you use
multiple Claude Code sessions, you can move narration from one session to
another by invoking `/attend` in whichever session you'd like to switch to.

To stop narration being delivered to this agent, use the `/unattend`
slash-command, or ask the agent to stop listening (it will run `attend listen
--stop` on your behalf). The running background listener detects the change and
exits naturally.

You can narrate responses to the agent without leaving your editor — the only
time you need to switch is when the agent asks for keyboard input (plan
approval, multiple-choice questions, or permission prompts).

As a security precaution, the agent only sees editor context (cursors,
selections, file contents, diffs) from within its own working directory. If you
navigate to files elsewhere in your editor, the agent won't be able to follow
along. You can expand this with `include_dirs` in the config file (see
[Configuration](#configuration)).

## Narration hotkeys

Narration is controlled by four commands. You'll have the best experience if
these are bound to hotkeys accessible without leaving your editor (or other
applications of interest, like [browsers](#browser-integration), etc.).

| Command                 | Purpose                                                  |
|-------------------------|----------------------------------------------------------|
| `attend narrate toggle` | Start recording if idle, or send and stop recording      |
| `attend narrate start`  | Start recording if idle, or send and keep recording      |
| `attend narrate pause`  | Pause/resume recording                                   |
| `attend narrate yank`   | Stop recording, and copy to clipboard instead of sending |

### Zed

`attend install --editor zed` installs keybindings and tasks automatically:

| macOS  | Linux     | Task                     |
|--------|-----------|--------------------------|
| `⌘ ;` | `Super ;` | attend: toggle narration |
| `⌘ :` | `Super :` | attend: start narration  |
| `⌘ {` | `Super {` | attend: pause narration  |
| `⌘ }` | `Super }` | attend: yank narration   |

Reinstallation respects any keybinding changes you've made within Zed.

### Global hotkeys

If you use a hotkey manager that can assign commands to keys, you can bind
*global* hotkeys to the narrate subcommands. On macOS, you can [bind a global
keyboard shortcut to a script using the Shortcuts
app](https://support.apple.com/guide/shortcuts-mac/launch-a-shortcut-from-another-app-apd163eb9f95/mac).
Pre-made shortcuts for the above 4 `attend` actions are in [`shortcuts/`](shortcuts/);
open them on your Mac to install. You will still need to manually open the Shortcuts app
and edit each action to assign a keyboard shortcut, because the `.shortcut` format does
not provide a way to embed a keymapping.

### iTerm2

iTerm2 does not pick up macOS global hotkeys set using the above technique. To
use the same keybindings from iTerm2, add key mappings under Settings > Keys >
Key Bindings:

1. Click **+** to add a new binding.
2. Set the shortcut, action **Run Coprocess**, and the corresponding command.
3. Ensure it is marked as "Apply to current session" (the default).
4. Repeat for each shortcut you want.

| Shortcut | Command                              |
|----------|--------------------------------------|
| `⌘ ;`   | `~/.cargo/bin/attend narrate toggle` |
| `⌘ :`   | `~/.cargo/bin/attend narrate start`  |
| `⌘ {`   | `~/.cargo/bin/attend narrate pause`  |
| `⌘ }`   | `~/.cargo/bin/attend narrate yank`   |

Use the full path (`~/.cargo/bin/attend`) because iTerm2 coprocesses run under
`/bin/sh`, which does not have `~/.cargo/bin` (and thereby `attend`) in its
`$PATH`.

## Browser integration

To capture text selections from your browser and deliver them as narration
context:

```bash
attend install --browser firefox   # or: --browser chrome
```

For Firefox, this installs a native messaging host manifest and opens the signed
extension for installation. After clicking "Add" in Firefox, the extension
persists across restarts.

For Chrome, this installs a native messaging host manifest and writes an
unpacked extension to a persistent directory. You then load it manually: open
`chrome://extensions`, enable Developer mode, click "Load unpacked", and select
the directory printed by the install command.

When narration is active, text you select in the browser will be captured with
the page URL and title, and delivered to your agent alongside speech and editor
context.

## Shell integration

To capture shell commands (what you ran, exit status, duration) as narration
context:

```bash
attend install --shell fish   # or: --shell zsh
```

This installs hooks that fire on every command. When narration is active,
commands you run in that shell are captured and delivered alongside speech and
editor context, so the agent can see what you executed.

## Transcription model

Narration uses a local speech-to-text model: no audio leaves your machine. By
default, the first time you start recording, the model is automatically
downloaded from [Hugging Face](https://huggingface.co/models) and cached
locally.

Two engines are available:

| Engine | Default Model | Size | Notes |
|--------|---------------|------|-------|
| `parakeet` (default) | [Parakeet TDT 0.6B](https://huggingface.co/istupakov/parakeet-tdt-0.6b-v3-onnx) | ~1.2 GB | Better quality, multilingual, faster |
| `whisper` | [Whisper Small (GGML)](https://huggingface.co/ggerganov/whisper.cpp) | ~466 MB | Smaller, English only, slower |

To change the engine, see [Configuration](#configuration).

## Configuration

`attend` loads config from two sources, merged together:

- **Project**: `.attend/config.toml` in the current directory or any parent
  (closer files take precedence for scalar values; arrays are concatenated)
- **Global**: `~/.config/attend/config.toml`

All fields are optional:

```toml
engine = "parakeet"                        # transcription engine: "parakeet" or "whisper"
model = "/path/to/custom/model"            # custom model path (auto-downloaded if omitted)
include_dirs = ["/path/to/other/project"]  # additional dirs visible to the agent
archive_retention = "7d"                   # auto-prune old narrations ("forever" to disable)
clipboard_capture = true                   # capture clipboard changes (text and images)
daemon_idle_timeout = "5m"                 # how long daemon idles before auto-exit ("forever" to disable)
```

## Troubleshooting

Run `attend narrate status` to check that everything is wired up correctly. It
will report something like this:

```
Recording:      recording
Engine:         Parakeet TDT (model downloaded)
Idle timeout:   5m (default)
Session:        a33c5803-8369-430d-9acf-70f24a5ba2d4
Listener:       active
Editors:        zed (ok)
Shells:         fish (ok)
Browsers:       firefox (ok)
Accessibility:  ok
Clipboard:      enabled
Pending:        0 narration(s)
Archive:        424.0 KB

Paths:
  Cache:      ~/Library/Caches/attend
  Archive:    ~/Library/Caches/attend/narration/archive
  Lock:       ~/Library/Caches/attend/daemon/lock
  Config:     ~/.config/attend/config.toml
```

### Microphone and accessibility permissions (macOS)

The recording daemon needs **Microphone** access for speech capture and
**Accessibility** access for capturing text selections in other applications.
On macOS, these permissions are granted to the `attend` binary itself —
you grant each permission once, and it works regardless of which app
(Zed, iTerm2, Terminal, Shortcuts) triggered the hotkey.

**First-time setup:** the first time you start recording after installation
(or after updating `attend`), macOS will prompt you to grant Microphone
access. Grant it in the system dialog or in **System Settings > Privacy &
Security > Microphone**. For text selection capture, add `attend` in
**System Settings > Privacy & Security > Accessibility**.

The binary location is typically `~/.cargo/bin/attend`. You can verify
with `which attend`.

**After updating `attend`:** when the binary is replaced (by `cargo install`
or other means), macOS may invalidate the previous permission grants. If
narration stops capturing speech or text selections after an update:

1. Kill all running `attend` processes: `killall attend`.
2. Open **System Settings > Privacy & Security**.
3. Under **Microphone** and **Accessibility**, remove `attend` and re-add it.
4. Start narration again — a fresh daemon will pick up the new permissions.

The daemon checks accessibility permission once at startup. If you change
the permission while the daemon is running, you must restart it (`killall
attend`) for the change to take effect.

**If the permission prompt never appeared**, the daemon may have been blocked
silently. Try running `attend narrate toggle` directly in a terminal to
trigger the prompt.

## Uninstall

To remove everything:

```bash
attend uninstall
cargo uninstall attend
```

Or, specify a particular `--agent` or `--editor` for `attend uninstall` to
uninstall only the integrations for that.
