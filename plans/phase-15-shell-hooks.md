# Phase 15: Shell Hook Integration

Capture command execution context (command text, exit status, duration) from
fish and zsh during narration sessions. Events are staged to disk with UTC
timestamps and delivered inline with speech, editor snapshots, and external
selections when narration is flushed or stopped.

## Motivation

Developers spend significant time running commands in their terminal. Today,
attend can see terminal text selections (via accessibility/ext_capture) and
browser selections, but it has no structured knowledge of *what commands the
user ran* or *what happened*. "The user ran `cargo test` and it failed after
3.2 seconds" is valuable context that currently requires the user to describe
verbally.

Shell hooks capture this automatically: the command text when it starts, the
exit status and wall-clock duration when it finishes. These interleave
chronologically with speech and selections, giving the agent a timeline of
what the user did and said.

## Design overview

### New Event variant

```rust
enum Event {
    // ... existing variants ...

    /// A command executed in the user's shell.
    ShellCommand {
        /// UTC wall-clock time when the command started.
        timestamp: chrono::DateTime<chrono::Utc>,
        /// The shell (e.g. "fish", "zsh").
        shell: String,
        /// The command as typed by the user.
        command: String,
        /// Exit status (None if preexec-only, before the command completes).
        exit_status: Option<i32>,
        /// Wall-clock duration in seconds (None if preexec-only).
        duration_secs: Option<f64>,
    },
}
```

A single event type covers both the "command started" and "command finished"
cases. The preexec hook writes an event with `exit_status: None` and
`duration_secs: None`. The postexec hook writes a complete event. The merge
pipeline can deduplicate preexec/postexec pairs for the same command (keep the
postexec, which has richer data), or keep both if they bracket interesting
interleaved events.

### Event staging (same pattern as browser)

Shell hooks stage events the same way browser selections do: write timestamped
JSON files to a staging directory. The recording daemon collects them at
flush/stop time.

```
~/.cache/attend/shell-staging/<session_id>/<timestamp>.json
```

This mirrors the browser staging path
([`src/narrate.rs:84-88`](https://github.com/oxidecomputer/attend/blob/755627e/src/narrate.rs#L84-L88)).
Each file contains a `Vec<Event>` (typically one event). The filename encodes
the UTC timestamp for ordering, same format as browser staging
(`2026-02-23T22-42-28Z.json`).

**Key properties** (inherited from browser staging design):
- Events only staged while `record_lock_path()` exists (daemon is recording)
- Events only staged when `listening_session()` returns a session
- Files are not removed until narration is safely written to disk (crash-safe)
- Events predating the current recording period are filtered out during
  collection

### Staging command: `attend shell-hook`

```
attend shell-hook preexec --shell fish --command "cargo test"
attend shell-hook postexec --shell fish --command "cargo test" --exit-status 1 --duration 3.2
```

A lightweight CLI subcommand (analogous to `attend browser-bridge`) that:
1. Checks `record_lock_path().exists()` — exits immediately if not recording
2. Checks `listening_session()` — exits immediately if no session
3. Writes a `ShellCommand` event to `shell_staging_dir(session_id)`
4. Exits (zero overhead when not recording)

This is invoked by the shell hooks. It must be fast: no model loading, no
network, no blocking. The browser bridge
([`src/cli/browser_bridge.rs`](https://github.com/oxidecomputer/attend/blob/755627e/src/cli/browser_bridge.rs))
is the template.

### Delivery: inline with other events, on manual trigger only

Shell events are **not** delivered to the agent until the user triggers
narration flush or stop (same as browser selections and all other capture
streams). The recording daemon's `collect_shell_staging()` (modeled on
`collect_browser_staging()` at
[`src/narrate.rs:125-174`](https://github.com/oxidecomputer/attend/blob/755627e/src/narrate.rs#L125-L174))
scans the staging directory and merges events into the event stream by UTC
timestamp. They interleave naturally with Words, EditorSnapshot, FileDiff,
ExternalSelection, and BrowserSelection.

### Markdown rendering

Shell events render as fenced code blocks with the shell name as the language
tag. This makes them copy-pasteable and syntax-highlighted. Exit status and
duration are reported as a trailing shell comment, so the block remains a
valid shell command.

**Postexec (command completed, non-trivial):**

```fish
cargo test  # exit 1, 3.2s
```

**Postexec (fast success — exit 0, < 1s — omit comment for token efficiency):**

```fish
cargo fmt
```

**Preexec (command started, still running at flush time):**

```fish
cargo test
```

Preexec and postexec for the same command interleave by timestamp with other
events (speech, selections). If both appear in the same run with no Words
between them, the merge pipeline keeps only the postexec (richer data). If
speech or other events fall between them, both survive — the preexec marks
when the user kicked off the command, and the postexec marks when it finished.

---

## Shell trait

### Trait definition

```rust
/// src/shell.rs

/// A shell integration that can install/uninstall hooks and completions.
pub trait Shell: Sync {
    /// CLI name (e.g., "fish", "zsh").
    fn name(&self) -> &'static str;

    /// Install shell hooks for narration capture.
    ///
    /// Writes hook files and prints the user's required config change
    /// (e.g., the `source` or `eval` line for their rc file).
    fn install_hooks(&self, bin_cmd: &str) -> anyhow::Result<()>;

    /// Remove shell hooks.
    fn uninstall_hooks(&self) -> anyhow::Result<()>;

    /// Install shell completions.
    fn install_completions(&self, bin_cmd: &str) -> anyhow::Result<()>;

    /// Remove shell completions.
    fn uninstall_completions(&self) -> anyhow::Result<()>;

    /// Check the health of the shell integration.
    /// Returns a list of diagnostic warnings (empty = healthy).
    fn check(&self) -> anyhow::Result<Vec<String>> {
        Ok(Vec::new())
    }
}

/// All registered shell backends.
pub const SHELLS: &[&'static dyn Shell] = &[
    &fish::Fish,
    &zsh::Zsh,
    // <-- Add new shells here
];

/// Look up a shell by CLI name.
pub fn shell_by_name(name: &str) -> Option<&'static dyn Shell> {
    SHELLS.iter().find(|s| s.name() == name).copied()
}
```

This follows the Browser trait pattern
([`src/browser.rs`](https://github.com/oxidecomputer/attend/blob/755627e/src/browser.rs)).
Hooks and completions are separate methods because a user might want
completions without narration hooks, or vice versa.

### Module layout

```
src/
  shell.rs              — Shell trait, registration, lookup
  shell/
    fish.rs             — Fish implementation
    zsh.rs              — Zsh implementation
```

---

## Fish implementation

### Hook installation

`attend install --shell fish` writes a hook file to
`~/.config/fish/conf.d/attend.fish`:

```fish
# Installed by attend. Do not edit; reinstall with: attend install --shell fish

function __attend_preexec --on-event fish_preexec
    # Only invoke the binary if the record lock exists (fast path).
    test -f ~/.cache/attend/record.lock; or return
    command attend shell-hook preexec --shell fish --command "$argv"
end

function __attend_postexec --on-event fish_postexec
    set -l __attend_status $status
    set -l __attend_duration $CMD_DURATION
    test -f ~/.cache/attend/record.lock; or return
    command attend shell-hook postexec \
        --shell fish \
        --command "$argv" \
        --exit-status $__attend_status \
        --duration (math "$__attend_duration / 1000")
end
```

**Key details:**
- `fish_preexec` receives the command text as `$argv`
- `fish_postexec` receives the command text as `$argv`; `$status` and
  `$CMD_DURATION` (milliseconds) are captured before any other code runs
- The `test -f record.lock` guard avoids spawning a process on every command
  when not recording (zero overhead in the common case)
- `conf.d/` files are automatically sourced by fish — no manual rc edit needed
- Double-underscore prefix prevents namespace collisions

### Completion installation

`attend install --shell fish` also writes completions to
`~/.config/fish/completions/attend.fish`, generated by clap's
`clap_complete::generate()` (attend already has a `Completions` subcommand at
[`src/cli.rs:146-151`](https://github.com/oxidecomputer/attend/blob/755627e/src/cli.rs#L146-L151)).

### Uninstallation

`attend uninstall --shell fish` removes:
- `~/.config/fish/conf.d/attend.fish`
- `~/.config/fish/completions/attend.fish`

---

## Zsh implementation

### Hook installation

`attend install --shell zsh` writes a hook file and instructs the user to
source it. Zsh doesn't have a `conf.d/` auto-source mechanism, so the user
must add a line to `~/.zshrc`.

**Hook file** (`~/.config/attend/hooks/attend.zsh`):

```zsh
# Installed by attend. Do not edit; reinstall with: attend install --shell zsh

__attend_preexec() {
    # $1 is the command string (from zsh's preexec hook).
    [[ -f ~/.cache/attend/record.lock ]] || return
    command attend shell-hook preexec --shell zsh --command "$1"
}

__attend_precmd() {
    local __attend_status=$?
    local __attend_end=$EPOCHREALTIME
    [[ -f ~/.cache/attend/record.lock ]] || return
    if [[ -n "$__attend_cmd" ]]; then
        local __attend_duration
        __attend_duration=$(( __attend_end - __attend_start ))
        command attend shell-hook postexec \
            --shell zsh \
            --command "$__attend_cmd" \
            --exit-status $__attend_status \
            --duration $__attend_duration
        unset __attend_cmd __attend_start
    fi
}

__attend_record_start() {
    __attend_cmd="$1"
    __attend_start=$EPOCHREALTIME
}

autoload -Uz add-zsh-hook
add-zsh-hook preexec __attend_record_start
add-zsh-hook preexec __attend_preexec
add-zsh-hook precmd __attend_precmd
```

**Key details:**
- Zsh's `preexec` passes the command string as `$1`
- Zsh's `precmd` runs before each prompt — captures `$?` for the previous
  command's exit status
- `$EPOCHREALTIME` gives sub-second wall-clock timing without forking `date`
- `add-zsh-hook` allows multiple hooks to coexist (doesn't clobber other tools)
- Duration is computed in zsh (preexec records start, precmd computes delta)

**User instruction** (printed by `attend install --shell zsh`):

```
Add this line to your ~/.zshrc:

    source ~/.config/attend/hooks/attend.zsh
```

### Completion installation

Writes `_attend` to `~/.config/attend/completions/` and instructs the user
to add the directory to `$fpath` before `compinit`:

```
Add this line to your ~/.zshrc (before compinit):

    fpath=(~/.config/attend/completions $fpath)
```

Or: generate completions inline and let the user pipe them wherever they want.

### Uninstallation

`attend uninstall --shell zsh` removes:
- `~/.config/attend/hooks/attend.zsh`
- `~/.config/attend/completions/_attend`
- Prints reminder to remove the `source` and `fpath` lines from `~/.zshrc`

---

## CLI integration

### Install / uninstall

```
attend install --shell fish
attend install --shell zsh
attend install --shell fish --shell zsh   # both at once
attend uninstall --shell fish
```

Follows the existing `--browser` pattern. Add `--shell` to the Install and
Uninstall commands in `src/cli.rs`, validated by a `shell_value_parser()` in
`src/cli/hook.rs` (same pattern as `browser_value_parser()`).

Update `InstallMeta` in `src/state.rs` to track installed shells:

```rust
pub struct InstallMeta {
    // ... existing fields ...
    #[serde(default)]
    pub shells: Vec<String>,
}
```

### Staging subcommand

```
attend shell-hook preexec --shell <shell> --command <command>
attend shell-hook postexec --shell <shell> --command <command> --exit-status <n> --duration <secs>
```

Hidden from help output (like `browser-bridge`). Designed for shell hooks to
call, not humans.

---

## Merge and compression

Add to `process_run` in `merge.rs`:
- `ShellCommand` events pass through without compression (each command is
  unique and meaningful)
- Preexec/postexec dedup: if a preexec and postexec for the same command text
  are in the same run, keep only the postexec (it has richer data)

Add to `compress_and_merge`: `ShellCommand` is a non-Words event, so it goes
into runs between Words boundaries and interleaves by timestamp.

---

## Collection by the recording daemon

Add `shell_staging_dir()` and `collect_shell_staging()` to `src/narrate.rs`,
following the exact pattern of `browser_staging_dir()` and
`collect_browser_staging()`. Wire into `DaemonState::collect_shell_staging()`
and pass through `transcribe_and_write()` alongside browser events.

Add `ShellStaging` and `ShellCleanup` types mirroring `BrowserStaging` and
`BrowserCleanup` for crash-safe deferred cleanup.

Or, more likely: **generalize the staging infrastructure**. Browser and shell
staging are identical except for the directory name. Consider extracting a
`StagingDir` helper:

```rust
struct StagingDir { path: Utf8PathBuf }

impl StagingDir {
    fn collect(&self, period_start_utc: DateTime<Utc>) -> (Vec<Event>, Vec<PathBuf>);
    fn cleanup(files: Vec<PathBuf>);
}
```

This avoids duplicating the collect/cleanup logic.

---

## Task breakdown

| # | Task | Depends On |
|---|------|------------|
| 1 | `Event::ShellCommand` variant + serde | — |
| 2 | `Shell` trait, module layout, fish + zsh stubs | — |
| 3 | `attend shell-hook` CLI subcommand (staging) | 1 |
| 4 | Fish hook + completion installation | 2, 3 |
| 5 | Zsh hook + completion installation | 2, 3 |
| 6 | `shell_staging_dir` + `collect_shell_staging` (or generalized `StagingDir`) | 1 |
| 7 | Wire shell staging into recording daemon (`transcribe_and_write`) | 6 |
| 8 | `render.rs`: render ShellCommand events | 1 |
| 9 | `merge.rs`: preexec/postexec dedup within runs | 1 |
| 10 | `receive.rs`: pass ShellCommand through filter unchanged | 1 |
| 11 | CLI `--shell` wiring (install/uninstall/status) | 2 |
| 12 | Tests: staging, merge/compress/prop, render, install round-trip | All above |

Tasks 1-3 are the foundation. 4 and 5 are independent of each other. 6-10
can proceed in parallel once 1 lands. 11 and 12 tie everything together.

---

## Potential prerequisite: SQLite for attend state

The current persistence model is filesystem-as-database: `listening`,
`record.lock`, `receive.lock`, `version.json`, `latest.json` (editor state
cache), two staging directory trees (browser, shell), and the
pending/archive narration directories. Adding shell staging means yet another
directory tree with the same scan-parse-cleanup pattern.

A single SQLite database would consolidate all of this:
- Atomic multi-row writes (no more crash-safety gymnastics with staging files)
- Single file to manage instead of a directory tree per event source
- Efficient querying (e.g., "events since timestamp X" without scanning files)
- Natural home for session state, install metadata, and editor state cache
- We already depend on `rusqlite` (for reading Zed's DB)

**Tradeoff**: more upfront work, and the current filesystem approach is
well-tested. But adding a third staging directory (shell) alongside browser
is the point where the duplication starts to hurt. If we're going to do this,
doing it before shell hooks land (or as part of the same phase) avoids
building more infrastructure on the filesystem pattern only to migrate it
later.

This could be scoped as phase 15a (SQLite migration) followed by 15b (shell
hooks on the new foundation), or deferred if the filesystem approach remains
acceptable.

---

## Open questions, with user feedback

1. **Should we capture command output?** This plan captures metadata only
   (command text, exit status, duration). Capturing stdout/stderr would
   require PTY wrapping (`attend wrap cargo test`) which is significantly
   more complex. Defer to a future phase.

   > Agreed, and command output might be enormous, so the user
   should manually select it when it's relevant.

2. **Preexec events: useful or noise?** If the command completes before
   narration flushes, the postexec event supersedes the preexec. Preexec is
   only useful for long-running commands still executing at flush time.
   Consider omitting preexec entirely and only capturing postexec.

   > I think the interleaved timing matters for narration, so we should preserve
   both. We should be token-efficient, though: omitting timing when it's small,
   and specifying the shell by flagging the code block language. Timing and
   exit code should be reported with a shell comment, so that the whole block
   is a valid shell command block.

3. **Bash support?** Bash lacks native preexec/postexec. The `bash-preexec`
   library fills the gap but is a third-party dependency the user must install.
   Defer to a later iteration if there's demand.

   > Bash doesn't even exist on macOS really anymore. Let's not bother.

4. **Should we generalize staging?** Browser and shell staging are structurally
   identical. Extracting a shared `StagingDir` reduces duplication but adds
   abstraction. Worth doing if we anticipate more staging sources.

   > Yes. We should also consider this potentially as an opportunity to
   reconsider our persistence mechanism: would a SQLite database of our own
   be more performant and easier to work with long term than our growing
   collection of various kinds of cache files?
