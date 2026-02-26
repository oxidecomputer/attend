# Setup guide

This guide covers optional integrations, configuration, and troubleshooting. For
initial installation, see the [quickstart in the
README](../README.md#quick-start).

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

## Narration hotkeys

Narration is controlled by four commands. You'll have the best experience if
these are bound to hotkeys accessible without leaving your editor.

| Command | Purpose |
|---------|---------|
| `attend narrate toggle` | Start narration, or send and stop |
| `attend narrate start` | Start narration, or send and keep recording |
| `attend narrate pause` | Pause/resume recording |
| `attend narrate yank` | Stop and copy narration to clipboard |

### Zed

`attend install --editor zed` installs keybindings and tasks automatically:

| macOS | Linux | Task |
|-------|-------|------|
| `⌘ ;` | `Super ;` | attend: toggle narration |
| `⌘ :` | `Super :` | attend: start narration |
| `⌘ {` | `Super {` | attend: pause narration |
| `⌘ }` | `Super }` | attend: yank narration |

Reinstallation respects any keybinding changes you've made in Zed.

### Global hotkeys (macOS / Linux)

If you use a hotkey manager that can assign commands to keys, you can bind
*global* hotkeys to the narrate subcommands. On macOS, you can [bind a global
keyboard shortcut to a script using the Shortcuts
app](https://support.apple.com/guide/shortcuts-mac/launch-a-shortcut-from-another-app-apd163eb9f95/mac).
Your favorite Linux distribution almost certainly has some way to do this too.

### iTerm2

iTerm2 does not pick up macOS global hotkeys. To use the same keybindings from
iTerm2, add key mappings under Settings > Keys > Key Bindings:

1. Click **+** to add a new binding.
2. Set the shortcut, action **Run Coprocess**, and the corresponding command.
3. Ensure it is marked as "Apply to current session".
4. Repeat for each shortcut you want.

| Shortcut | Command |
|----------|---------|
| `⌘;` | `~/.cargo/bin/attend narrate toggle` |
| `⌘:` | `~/.cargo/bin/attend narrate start`  |
| `⌘{` | `~/.cargo/bin/attend narrate pause`  |
| `⌘}` | `~/.cargo/bin/attend narrate yank`   |

Use the full path (`~/.cargo/bin/attend`) because iTerm2 coprocesses run under
`/bin/sh`, which does not have `~/.cargo/bin` in its PATH.

## Agent integration

For the agent to receive your narration, ask it to attend.

In Claude Code, this is done with the `/attend` slash-command. If you use
multiple Claude Code sessions, you can move narration from one session to
another by invoking `/attend` in whichever session you'd like to switch to.

Insofar as the agent doesn't ask for keyboard input (i.e. by presenting a plan,
asking a multiple choice question, or requesting permission to do an action),
you need never leave your focused editor, because you can narrate your responses
while you're in the codebase.

As a security precaution, the agent only sees editor context (cursors,
selections, file contents, diffs) from within its own working directory. If you
navigate to files elsewhere in your editor, the agent won't be able to follow
along. You can expand this with `include_dirs` in the config file (see
[Configuration](#configuration)).

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
```

## Troubleshooting

Run `attend narrate status` to check that everything is wired up correctly. It
shows whether narration is recording, which engine and session are active,
whether the editor integration is healthy, and whether any narration is pending.

### Microphone permissions (macOS)

The recording daemon needs microphone access. macOS prompts for permission the
first time the daemon is launched from a given parent process. If narration
starts but produces no speech, check **System Settings > Privacy & Security >
Microphone** and ensure the **focused app** has microphone permissions, because
the keyboard shortcut is triggering the script from within that app's context.

If the permission prompt never appeared, the daemon may have been blocked
silently. Try running `attend narrate toggle` directly in a terminal to trigger
the prompt.

## Uninstall

To remove everything:

```bash
attend uninstall
cargo uninstall attend
```

Or, specify a particular `--agent` or `--editor` for `attend uninstall` to
uninstall only the integrations for that.
