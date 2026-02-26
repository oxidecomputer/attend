# Adding a new shell

A shell backend installs hooks that fire on every command (preexec/postexec)
and stage events for the narration pipeline. It also installs tab completions.

## How shell hooks work

When narration is recording, the installed shell hook calls `attend shell-hook`
on each command:

1. **Preexec** (command starting): `attend shell-hook preexec --shell <name> --command "..."`.
2. **Postexec** (command completed): `attend shell-hook postexec --shell <name> --command "..." --exit-status 0 --duration 2.5`.

The CLI handler (`src/cli/shell_hook.rs`) checks the record lock (fast no-op
when not recording), resolves the active session, and atomically writes a
`ShellCommand` event to the shell staging directory. The recording daemon
collects these events and merges them chronologically into the narration.

## 1. Create the module — `src/shell/<name>.rs`

Implement the `Shell` trait:

```rust
pub struct Name;

impl Shell for Name {
    fn name(&self) -> &'static str { "<name>" }

    fn install_hooks(&self, bin_cmd: &str) -> anyhow::Result<()> {
        // Write a hook script that calls:
        //   {bin_cmd} shell-hook preexec --shell <name> --command "$cmd"
        //   {bin_cmd} shell-hook postexec --shell <name> --command "$cmd" \
        //       --exit-status $status --duration $secs
        //
        // The hook should check `record_lock_path().exists()` as a fast
        // path to skip work when not recording.
        //
        // If the shell doesn't auto-source from a config directory,
        // print the `source` line the user needs to add to their rc file.
        Ok(())
    }

    fn uninstall_hooks(&self) -> anyhow::Result<()> {
        // Remove the hook script written by install_hooks().
        Ok(())
    }

    fn install_completions(&self, bin_cmd: &str) -> anyhow::Result<()> {
        // Generate completions via clap_complete::generate() and write
        // to the shell's completions directory.
        Ok(())
    }

    fn uninstall_completions(&self) -> anyhow::Result<()> {
        // Remove the completions file.
        Ok(())
    }

    fn check(&self) -> anyhow::Result<Vec<String>> {
        // Optional: return diagnostic warnings (empty = healthy).
        Ok(Vec::new())
    }
}
```

### `Shell` trait methods

| Method                  | Required | Purpose                                          |
|-------------------------|----------|--------------------------------------------------|
| `name()`                | yes      | CLI name, e.g. `"fish"`                          |
| `install_hooks()`       | yes      | Write hook script that calls `attend shell-hook`  |
| `uninstall_hooks()`     | yes      | Remove the hook script                           |
| `install_completions()` | yes      | Generate and write tab completions               |
| `uninstall_completions()`| yes     | Remove the completions file                      |
| `check()`               | no       | Return diagnostic warnings (empty = healthy)     |

## 2. Register the backend in `src/shell.rs`

Add the module and register it in the `SHELLS` slice:

```rust
mod fish;
mod zsh;
mod <name>;
```

```rust
pub const SHELLS: &[&dyn Shell] = &[
    &fish::Fish,
    &zsh::Zsh,
    &<name>::Name,
];
```

The CLI (`install --shell <name>`, `uninstall --shell <name>`) is built
automatically from the registered backends.

## Implementation notes

- **Fast path**: Hook scripts should check
  `~/.local/share/attend/record.lock` (the record lock) before spawning
  `attend`. This makes the hook free when narration is inactive. See
  `src/shell/fish.rs` for the fish pattern.
- **Auto-sourcing**: Fish hooks go in `~/.config/fish/conf.d/` and are
  loaded automatically. Zsh hooks go in `~/.config/attend/hooks/` and
  require a `source` line in `~/.zshrc` (printed by the installer). Choose
  whichever pattern your shell supports.
- **Duration**: The postexec hook should report command duration in seconds.
  Fish provides `$CMD_DURATION` (milliseconds); zsh uses `$EPOCHREALTIME`
  deltas. Use whatever mechanism your shell exposes.
- **Idempotency**: `install_hooks()` must be safe to call repeatedly.

## Checklist

- [ ] `src/shell/<name>.rs` — `pub struct Name` + `impl Shell for Name`
- [ ] `src/shell.rs` — `mod <name>;` declaration
- [ ] `src/shell.rs` — add `&<name>::Name` to `SHELLS`
