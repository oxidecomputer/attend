# How to configure narration hotkeys

Narration is controlled by four commands. Bind them to hotkeys you can reach
without leaving your editor or other applications.

| Command                 | Purpose                                                    |
|-------------------------|------------------------------------------------------------|
| `attend narrate toggle` | Start recording if idle, or send and stop recording        |
| `attend narrate start`  | Start recording if idle, or send and keep recording        |
| `attend narrate pause`  | Pause/resume recording *without* sending                   |
| `attend narrate yank`   | Stop recording, and copy to clipboard *instead of* sending |

## Zed

`attend install --editor zed` installs keybindings and tasks automatically:

| macOS  | Linux     | Task                     |
|--------|-----------|--------------------------|
| `⌘ ;`  | `Super ;` | attend: toggle narration |
| `⌘ :`  | `Super :` | attend: start narration  |
| `⌘ {`  | `Super {` | attend: pause narration  |
| `⌘ }`  | `Super }` | attend: yank narration   |

Reinstallation respects any keybinding changes you've made within Zed.

Editor-only hotkeys mean you can only control narration when the editor is
focused. If you also want control from your browser, terminal, or other
applications, set up global hotkeys.

## Global hotkeys (macOS)

If you use a hotkey manager, bind global hotkeys to the narrate subcommands. On
macOS, you can [bind a global keyboard shortcut to a script using the Shortcuts
app](https://support.apple.com/guide/shortcuts-mac/launch-a-shortcut-from-another-app-apd163eb9f95/mac).

Pre-made shortcuts for the four actions are in [`shortcuts/`](../../shortcuts/);
open them on your Mac to install. You will still need to open the Shortcuts app
and edit each action to assign a keyboard shortcut — the `.shortcut` format
does not provide a way to embed a keymapping.

## iTerm2

iTerm2 does not pick up macOS global hotkeys set using the Shortcuts technique
above. To use the same keybindings from iTerm2, add key mappings under Settings
> Keys > Key Bindings:

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
`/bin/sh`, which does not have `~/.cargo/bin` in its `$PATH`.
