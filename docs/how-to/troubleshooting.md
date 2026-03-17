# Troubleshooting

## Check system status

Run `attend narrate status` to see if everything is wired up:

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

## Common issues

### "model not yet downloaded"

Normal on first run. The model downloads automatically when you first run
`/attend` in your agent, or you can download it manually with `attend narrate
model download`.

### "Accessibility: not granted"

Add `attend` in System Settings > Privacy & Security > Accessibility. See
[macOS permissions](#macos-permissions) below.

### Narration not arriving

Check that **Listener** shows `active` in `attend narrate status`. If not, run
`/attend` (or `/attend:start` with the plugin) in your agent session.

### Narration arriving in the wrong session

Run `/attend` (or `/attend:start`) in the session you want to receive
narration. The previous session releases ownership automatically.

### Microphone not capturing

macOS may have silently denied microphone access. Try running `attend narrate
toggle` directly in a terminal to trigger the permission prompt.

### Transcription quality is poor

- **Noisy environment**: The voice activity detector may struggle in noisy
  settings. Move closer to the microphone or use a directional mic/headset.
- **Wrong microphone**: The daemon uses the system default input device. On
  macOS, check **System Settings > Sound > Input** to verify the correct
  microphone is selected.
- **Try the other engine**: If using Whisper, switch to Parakeet (or vice
  versa) in your [configuration](../reference/configuration.md#fields). Parakeet generally
  produces better results, and so is the default.

### Daemon won't start

If `attend narrate toggle` appears to do nothing:

1. Check for a stale lock file: `ls ~/Library/Caches/attend/daemon/lock`. If it
   exists but no `attend` process is running (`pgrep attend`), remove it:
   `rm ~/Library/Caches/attend/daemon/lock`.
2. Run `attend narrate toggle` in a terminal to see any error output directly.
3. Verify the transcription model is downloaded: `attend narrate status` should
   show "model downloaded" next to the engine.

## macOS permissions

The recording daemon needs **Microphone** access for speech capture and
**Accessibility** access for capturing text selections in other applications.
These permissions are granted to the `attend` binary itself — you grant each
permission once, and it works regardless of which app triggered the hotkey.

### First-time setup

The first time you start recording after installation (or after updating
`attend`), macOS will prompt you to grant Microphone access.

1. Grant it in the system dialog, or go to **System Settings > Privacy &
   Security > Microphone** and enable `attend`.
2. For text selection capture, go to **System Settings > Privacy & Security >
   Accessibility** and add `attend`.

The binary location is typically `~/.cargo/bin/attend`. Verify with `which
attend`.

### After updating attend

When the binary is replaced (by `cargo install` or other means), macOS may
invalidate the previous permission grants. If narration stops capturing speech
or text selections after an update:

1. Kill all running `attend` processes: `killall attend`.
2. Open **System Settings > Privacy & Security**.
3. Under **Microphone** and **Accessibility**, remove `attend` and re-add it.
4. Start narration again — a fresh daemon will pick up the new permissions.

The daemon checks accessibility permission once at startup. If you change the
permission while the daemon is running, restart it (`killall attend`) for the
change to take effect.
