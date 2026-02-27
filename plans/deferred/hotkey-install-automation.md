# Hotkey Install Automation

Deferred plan for `attend install --iterm2` and `attend install --hotkeys` (or
similar) to programmatically configure keyboard shortcuts for narration.

Status: **Deferred** (documented 2026-02-26, not yet prioritized)

## Context

Today, hotkey setup is manual and documented in `docs/setup.md`. Four commands
need keybindings:

| Shortcut | macOS  | Command                 |
|----------|--------|-------------------------|
| toggle   | `Cmd ;`  | `attend narrate toggle` |
| start    | `Cmd :` | `attend narrate start`  |
| pause    | `Cmd {` | `attend narrate pause`  |
| yank     | `Cmd }` | `attend narrate yank`   |

Two environments need shortcuts: a global hotkey layer (so shortcuts work from
any app, including Zed) and iTerm2 (which doesn't pick up macOS global hotkeys).

## iTerm2: fully automatable via `defaults write`

### Encoding

iTerm2 stores global key bindings in the `GlobalKeyMap` dict within
`com.googlecode.iterm2.plist`. Each entry's key is:

```
0x<CHAR>-0x<MODIFIERS>-0x<VIRTUAL_KEYCODE>
```

| Component        | Description                                    |
|------------------|------------------------------------------------|
| `0x<CHAR>`       | Unicode code point of the resulting character   |
| `0x<MODIFIERS>`  | Bitmask of NSEventModifierFlags                 |
| `0x<KEYCODE>`    | macOS virtual keycode (hardware key position)   |

Modifier flags:

| Modifier | Hex       |
|----------|-----------|
| Shift    | `0x20000`  |
| Control  | `0x40000`  |
| Option   | `0x80000`  |
| Command  | `0x100000` |

Combine with bitwise OR (e.g. Cmd+Shift = `0x120000`).

### Binding values

Each value is a dict:

```xml
<dict>
  <key>Action</key><integer>35</integer>
  <key>Apply Mode</key><integer>0</integer>
  <key>Escaping</key><integer>2</integer>
  <key>Text</key><string>~/.cargo/bin/attend narrate toggle</string>
  <key>Version</key><integer>2</integer>
</dict>
```

| Field      | Value | Meaning                           |
|------------|-------|-----------------------------------|
| Action     | 35    | KEY_ACTION_RUN_COPROCESS          |
| Apply Mode | 0     | Apply to current session          |
| Escaping   | 2     | Coprocess escaping mode           |
| Version    | 2     | Current iTerm2 binding schema     |

### Exact bindings

| Key string             | Shortcut | Command |
|------------------------|----------|---------|
| `0x3b-0x100000-0x29`   | Cmd+;    | toggle  |
| `0x3a-0x120000-0x29`   | Cmd+:    | start   |
| `0x7b-0x120000-0x21`   | Cmd+{    | pause   |
| `0x7d-0x120000-0x1e`   | Cmd+}    | yank    |

Virtual keycodes: `0x29` = kVK_ANSI_Semicolon, `0x21` = kVK_ANSI_LeftBracket,
`0x1e` = kVK_ANSI_RightBracket.

Note: when Shift is held, the first component reflects the shifted character
(`;` becomes `:`, `[` becomes `{`, `]` becomes `}`).

### Implementation

```bash
defaults write com.googlecode.iterm2 GlobalKeyMap -dict-add \
  "0x3b-0x100000-0x29" \
  '<dict><key>Action</key><integer>35</integer><key>Apply Mode</key><integer>0</integer><key>Escaping</key><integer>2</integer><key>Text</key><string>~/.cargo/bin/attend narrate toggle</string><key>Version</key><integer>2</integer></dict>'
```

Repeat for each binding. Also add the stderr-suppression entries:

```bash
defaults write com.googlecode.iterm2 NoSyncCoprocessCommandsToIgnoreErrorOutput \
  -array-add "~/.cargo/bin/attend narrate toggle"
# ... repeat for each command
```

### Caveats

- **cfprefsd caching**: if iTerm2 is running, it holds preferences in memory.
  `defaults write` will write to the plist on disk, but iTerm2 may overwrite it
  when it next saves. The new bindings won't take effect until iTerm2 is
  restarted. Nothing is corrupted or lost: worst case is the keys don't stick
  and the user reruns install after quitting iTerm2.
- **`-dict-add` is safe**: it merges into the existing GlobalKeyMap without
  destroying other entries.
- **Idempotency**: running install twice overwrites the same keys with the same
  values. No duplicates.

### Proposed flow

1. Write bindings with `defaults write`
2. Check if iTerm2 is running (`pgrep -x iTerm2`)
3. If running: print "Restart iTerm2 to pick up the new keybindings"
4. If not running: print "Done"

### Uninstall

Remove the four keys from `GlobalKeyMap`. This requires read-modify-write
(export, remove keys, import) since `defaults` has no `dict-remove`. Also
remove entries from `NoSyncCoprocessCommandsToIgnoreErrorOutput`.

---

## macOS Global Shortcuts: options evaluated

### Current setup (manual)

Uses the macOS Shortcuts app with four shortcuts, each containing a single "Run
Shell Script" action. Keyboard shortcuts are assigned via System Settings >
Keyboard > Keyboard Shortcuts > Services, and stored in `pbs` preferences under
`NSServicesStatus`:

```
"(null) - <WORKFLOW_UUID> - runShortcutAsService" = {
    key_equivalent = "@;";   // @ = Cmd, $ = Shift
};
```

### Why this can't be fully automated

The `shortcuts` CLI only supports `list`, `run`, `view`, and `sign`. There is
no `create` or `import` subcommand. The shortcuts are stored in a CoreData
SQLite database (`~/Library/Shortcuts/Shortcuts.sqlite`) which is fragile to
write to directly (schema versions, iCloud sync, CoreData triggers).

The keyboard shortcut bindings in `pbs` can be written with `defaults write`,
but they reference the Shortcut's workflow UUID, which only exists after the
Shortcut is created in the app.

#### `.shortcut` file distribution (investigated, dead end)

`.shortcut` files are a real distribution format: binary plists containing
`WFWorkflowActions` (action identifier + parameters), icon, and metadata. We
can construct them trivially — the action for "Run Shell Script" is
`is.workflow.actions.runshellscript` with a `Script` parameter. However:

- **Unsigned `.shortcut` files are rejected** by macOS on import ("Importing
  unsigned shortcut files is not supported").
- **`shortcuts sign` is broken on macOS 14.4+** (Sonoma and later). It fails
  with `Unrecognized attribute string flag '?' in attribute string` errors in
  Apple's own signing infrastructure. This is a known Apple bug
  ([cherri#49](https://github.com/electrikmilk/cherri/issues/49)).
- The only workaround is a third-party signing service (HubSign/RoutineHub),
  which is not a dependency we'd want.
- Even if signing worked, importing a `.shortcut` opens a GUI confirmation
  dialog per file (not scriptable), and doesn't assign keyboard shortcuts.

### Alternative approaches evaluated

#### Automator Quick Actions (`.workflow` bundles)

- `.workflow` bundles are directories with a known structure; can be created
  programmatically and placed in `~/Library/Services/`.
- Keyboard shortcuts can be assigned via `defaults write pbs NSServicesStatus`.
- **Problem**: fragile on modern macOS. Apple has progressively locked down the
  `pbs.plist` path; writes may not persist. Automator is deprecated (though
  still functional on Sequoia).
- **Verdict**: possible but unreliable.

#### Hammerspoon

- Lua config in `~/.hammerspoon/init.lua`. Install via `brew install hammerspoon`.
- Two lines per binding:
  ```lua
  hs.hotkey.bind({"cmd"}, ";", function()
      hs.task.new("/Users/oxide/.cargo/bin/attend", nil, {"narrate", "toggle"}):start()
  end)
  ```
- Config can be generated/appended programmatically. Hot-reloads with
  `hs -c "hs.reload()"`.
- Requires Accessibility permission (one-time manual grant, unavoidable macOS
  security requirement).
- Actively maintained, lightweight, pure userspace.
- **Verdict**: best third-party option. Only manual step is Accessibility
  permission.

#### skhd

- Config in `~/.skhdrc`:
  ```
  cmd - 0x29 : ~/.cargo/bin/attend narrate toggle
  ```
- Install via `brew install koekeishiya/formulae/skhd`.
- Uses virtual keycodes for non-alphanumeric keys (`0x29` for semicolon).
- Requires Accessibility permission. Also requires "Secure Keyboard Entry" to
  be disabled in Terminal.app.
- **In maintenance mode** (critical bugs only). Recurring macOS compatibility
  issues on Sequoia.
- **Verdict**: viable but risky given maintenance status.

#### Karabiner-Elements

- JSON config in `~/.config/karabiner/karabiner.json`, hot-reloads on change.
- Uses `shell_command` in complex modifications:
  ```json
  {
      "from": { "key_code": "semicolon", "modifiers": { "mandatory": ["command"] } },
      "to": [{ "shell_command": "/Users/oxide/.cargo/bin/attend narrate toggle" }]
  }
  ```
- Requires Input Monitoring permission + system extension approval.
- Actively maintained, large user base. Heavier permission model.
- Shell commands run under `/bin/sh` with minimal `$PATH`: must use absolute
  paths.
- **Verdict**: strong option, especially for users who already have Karabiner.

#### Custom Swift helper app

- Small Swift binary using Carbon `RegisterEventHotKey` API, installed as a
  login item.
- Maximum control, no runtime dependencies.
- Requires Accessibility permission. Must be signed (or ad-hoc signed).
- **Verdict**: most work to build and maintain. Not worth it unless we want zero
  third-party dependencies.

### Summary

| Approach          | Auto-  | Extra    | Reboot | Reliability | Permissions       |
|                   | matable| Software | Safe   |             |                   |
|-------------------|--------|----------|--------|-------------|-------------------|
| Shortcuts (now)   | No     | None     | Yes    | High        | Minimal           |
| Automator + pbs   | Fragile| None     | ?      | Low         | Minimal           |
| Hammerspoon       | Config | brew     | Yes    | High        | Accessibility     |
| skhd              | Config | brew     | Yes    | Medium      | Accessibility     |
| Karabiner         | Config | brew     | Yes    | High        | Input Monitoring  |
| Custom Swift app  | Build  | Xcode    | Yes    | High        | Accessibility     |

### Recommendation

If we pursue this, **Hammerspoon** is the lightest-weight third-party option.
But it's a new dependency for users who don't already have it. The current
manual Shortcuts approach works well and requires no additional software. This
is worth revisiting if users report setup friction.
