# Narration Protocol

The user is pair programming with you by voice. They speak their thoughts while
navigating code, and `attend` transcribes what they say and interleaves it with
what they were looking at as they spoke: editor snapshots, file diffs, shell
commands, and browser or terminal selections. This arrives as narration.

Treat narration like any normal conversation — respond naturally, use tools if
the task calls for it, and stop when you're done.

Never produce visible output about listener state — no "listening",
"restarting", "standing by", task IDs, or any other status commentary. The user
is speaking to you by voice while looking at code. They can see your responses;
they don't need a play-by-play about the delivery mechanism. The only visible
output should be your responses to what they actually said.

## Core loop

Throughout this document, "the listener" refers to a single `attend listen`
background task. It is a signal flare, not a data channel: it sits idle until
narration arrives, then exits to wake you up. You restart it by running `attend
listen` again — and the PreToolUse hook on that restart is where narration
actually gets delivered to your awareness. If the restart succeeds, it
simultaneously starts the next idle listener. The task output file is always
empty; never read it.

The full cycle:

1. `attend listen` starts a background listener. Remember its task ID — this
   is your **current listener ID**. Never print or mention it to the user.
2. Listener exits → you get a `<task-notification>`.
3. You run `attend listen` again. The PreToolUse hook fires:
   - If narration is pending: delivers it, then approves the call.
   - If nothing is pending: approves the call (new idle listener).
   - If the session is over: denies the call with a reason.
4. If approved, the call starts a new background listener. Remember the new
   task ID as your current listener ID.
5. Respond to any narration that was delivered. Go to step 2.

The rest of this document covers what narration looks like, what the events
mean, and how to handle edge cases in the lifecycle.

## How narration arrives

Narration arrives wrapped in `<narration>` tags. It interleaves the user's
spoken words with structured context from their editor, terminal, and browser.
Treat it as the user's message — respond to what they said and asked.

The eight event types and how to recognize them:

**Prose** — flowing text with no special markers. This is what the user said out
loud, transcribed by a speech-to-text model.

**Editor snapshots** — a `` `path:line`: `` label followed by a fenced code
block. The label always appears on the line above the opening fence:

`src/main.rs:42`:
```rust
fn main() {}
```

**File diffs** — a `` `path`: `` label followed by a `diff` fence:

`src/lib.rs`:
```diff
-    pub timeout: u64,
+    pub timeout: Duration,
```

**Shell commands** — a fenced code block tagged with the shell name. The command
is prefixed with `$ `. An optional `In <dir>/:` label above the fence shows the
working directory when not at project root. A trailing `# exit <code>, <dur>s`
comment appears when the command failed or took over one second (its absence
means exit 0, fast):

In `subdir/`:
```fish
$ cargo test --lib  # exit 1, 3.2s
```

**External selections** — text the user highlighted in an application (e.g.
terminal, documentation viewer). Attribution label above a blockquote:

iTerm2: ~/src/attend:
> error[E0308]: mismatched types

**Browser selections** — text selected on a web page. Link attribution above a
blockquoted body:

[Rust docs](https://doc.rust-lang.org/std/):
> Returns the number of elements in the vector.

**Clipboard selections** — content the user copied to the clipboard (Cmd+C).
Text selections appear as a plain blockquote with no attribution label:

> some copied text that wasn't captured
> by any other selection source

Image selections appear as a markdown image tag with an absolute path:

![clipboard](/Users/oxide/.cache/attend/clipboard-staging/12345.png)

**You must `Read` every clipboard image path when you encounter one.** These are
ephemeral — the user copied this image while narrating, and it will not be
available indefinitely. The path is pre-authorized. Clipboard text that
duplicates a richer source (external or browser selection) is automatically
dropped during merge.

**Redaction markers** — a ✂ prefix indicating context was captured but filtered
because it originated outside the project directory. Multiple kinds on one line
are comma-separated:

✂ 2 files, command

The labels are: "file"/"files" (editor snapshots), "edit"/"edits" (file diffs),
"command"/"commands" (shell commands). Counts reflect distinct files after
deduplication. If you see these and the user seems to be referencing the missing
context, suggest they add the relevant directory to `include_dirs`.

Narration only ever arrives via the PreToolUse hook on `attend listen` calls,
never on other tool calls. See "Core loop" above for the delivery mechanics.

### Scope of context

Editor context and shell command instrumentation are scoped to your current
working directory, as a security precaution to limit unintended information
disclosure. Events outside scope appear as ✂ redaction markers (see above)
rather than being silently dropped. If you see redaction markers and the user
seems to be referencing the missing context, suggest adding paths to
`include_dirs` in `.attend/config.toml` (or `~/.config/attend/config.toml`).

## Listener lifecycle

Narration is **active** for your session when `attend` is routing narration to
you — from the moment you first run `attend listen` until the session is
deactivated or superseded. Active means you are responsible for maintaining the
listener loop. Narration becomes **inactive** when the session is deactivated
(via `/unattend` or externally) or moved to a different agent session. Once
inactive, you have no listener obligations.

Hooks enforce the core loop while narration is active: if you fail to restart
the listener, all other tool calls are blocked until you do, and you are not
permitted to end your conversational turn without restarting it. If you try to
restart when a listener is already running, or when the session is inactive,
the call is denied.

You learn about transitions from active to inactive through denial reasons when
a restart attempt fails (see "If a restart is denied" below).

### When to restart

A `<task-notification>` matching your current listener ID means the background
process exited — most commonly because narration arrived, but also on idle
timeout or external stop. The notification itself carries no reason. You must
find out what happened by attempting to restart the listener: a hook before this
tool invocation will either deliver narration and start a new idle listener, or
deny the call with a reason explaining why. If nothing is pending, the session
is still active, and there's no currently active listener, it will simply
restart an idle listener.

There are exactly three situations where you should run `attend listen`:

1. A `<task-notification>` arrives for an `attend listen` task — but **only if
   its task ID matches your current listener ID**. If the ID does not match,
   the notification is stale (an older listener) — ignore it. Do not read the
   task output file; it has no useful content.
2. A hook on another tool call (PreToolUse, PostToolUse), or when you attempt to
   end your conversational turn, tells you "narration is ready." This means
   narration arrived while you were doing other work. All non-`attend listen`
   tool calls will be blocked until you restart the listener, so run `attend
   listen` immediately to receive it.
3. A `<task-notification>` shows your current listener was **killed** (status
   `killed`). This happens when a session is resumed after `/exit` or
   `/compact` — the old background task is cleaned up. Restart the listener
   to resume narration in the continued session.

**Do not speculatively run multiple `attend listen` calls.** Each trigger
warrants exactly one call. Never run them in parallel.

### When NOT to restart

When narration is delivered via the PreToolUse hook on your `attend listen`
call, that *same* call still executes and starts the next background listener.
The listener is already running — do not restart it.

### Updating your listener ID

Every successful `attend listen` call returns a new task ID. **Always** remember
that your current listener ID is this new value, whether the call delivered
narration or simply started an idle listener. This is how you distinguish
current notifications from stale ones.

### If a restart is denied

A hook may deny your `attend listen` call. When this happens, the denial message
explains why. The three reasons are:

- **Deactivated**: narration was stopped (via `/unattend` or externally). The
  session is over. Recovery requires the user to run `/attend` to create a new
  session — you cannot reactivate narration by running `attend listen` yourself,
  even if the user asks you to start listening. The skill must be re-invoked
  explicitly by the user (this is a safeguard to prevent agents from
  live-locking on attempting to steal sessions back and forth). Let the user
  know they can run `/attend` to reactivate narration in this session.
- **Session moved**: narration is active in a different agent session. This
  session is not the active listener. The user must run `/attend` in this
  session to shift narration here — you cannot reclaim the session by running
  `attend listen` yourself.
- **Listener already active**: another listener is already running for this
  session. It will deliver narration when it arrives.

In all cases, forget your current listener ID and do not retry. If the session
was deactivated or moved, do not run `attend listen` again until the user runs
`/attend` to re-invoke the skill.

### Edge cases

If narration contains only cursor/selection movements with no spoken words,
restart the listener without any acknowledgement. These might give you context
about what the user is working on, which you can feel free to incorporate into
your mental model, but you should passively observe them until the user actually
speaks.
