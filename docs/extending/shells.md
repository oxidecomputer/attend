# How to add a new shell

A shell backend installs hooks that fire on every command (preexec/postexec)
and stage events for the narration pipeline. It also installs tab completions.
See [extending reference](reference.md#shell-trait) for the full
trait API.

## How shell hooks work

When narration is recording, the installed shell hook calls `attend shell-hook`
on each command:

1. **Preexec** (command starting): `attend shell-hook preexec --shell <name> --command "..."`.
2. **Postexec** (command completed): `attend shell-hook postexec --shell <name> --command "..." --exit-status 0 --duration 2.5`.

The CLI handler (`src/cli/shell_hook.rs`) checks the record lock (fast no-op
when not recording), resolves the active session, and atomically writes a
`ShellCommand` event to the shell staging directory. The recording daemon
collects these events and merges them into the narration.

## 1. Create the module

Create `src/shell/<name>.rs` implementing the `Shell` trait:

```rust
pub struct Name;

impl Shell for Name {
    fn name(&self) -> &'static str { "<name>" }

    fn install_hooks(&self, bin_cmd: &str) -> anyhow::Result<()> {
        // Write a hook script that calls:
        //   {bin_cmd} shell-hook preexec --shell <name> --command "$cmd"
        //   {bin_cmd} shell-hook postexec --shell <name> --command "$cmd" \
        //       --exit-status $status --duration $secs
        Ok(())
    }

    fn uninstall_hooks(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn install_completions(&self, bin_cmd: &str) -> anyhow::Result<()> {
        Ok(())
    }

    fn uninstall_completions(&self) -> anyhow::Result<()> {
        Ok(())
    }

    fn check(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}
```

## 2. Register the backend

In `src/shell.rs`, add the module and register it:

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

## Implementation notes

- **Fast path**: Hook scripts should check `record_lock_path()` (resolved at
  install time and baked into the script) before spawning `attend`. This makes
  the hook free when narration is inactive. See `src/shell/fish.rs` for the
  fish pattern.
- **Auto-sourcing**: Fish hooks go in `~/.config/fish/conf.d/` and are
  loaded automatically. Zsh hooks require a `source` line in `~/.zshrc`.
  Choose whichever pattern your shell supports.
- **Duration**: The postexec hook should report command duration in seconds.
  Fish provides `$CMD_DURATION` (milliseconds); zsh uses `$EPOCHREALTIME`
  deltas.
- **Idempotency**: `install_hooks()` must be safe to call repeatedly.

## Checklist

- [ ] `src/shell/<name>.rs` — `pub struct Name` + `impl Shell for Name`
- [ ] `src/shell.rs` — `mod <name>;` declaration
- [ ] `src/shell.rs` — add `&<name>::Name` to `SHELLS`
