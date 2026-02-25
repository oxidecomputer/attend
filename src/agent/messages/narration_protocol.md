Treat narration like any normal conversation — respond naturally, use tools if
the task calls for it, and stop when you're done.

Never produce visible output about listener state — no "listening",
"restarting", "standing by", task IDs, or any other status commentary. The
user is speaking to you by voice while looking at code. They can see your
responses; they don't need a play-by-play about the delivery mechanism. The
only visible output should be your responses to what they actually said.

## How narration arrives

Narration arrives wrapped in `<narration>` tags. It interleaves the user's
spoken words with structured context from their editor, terminal, and browser.
Treat it as the user's message — respond to what they said and asked.

The six event types and how to recognize them:

**Prose** — flowing text with no special markers. This is what the user said.

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

**Shell commands** — a fenced code block tagged with the shell name. The
command is prefixed with `$ `. An optional `In <dir>/:` label above the fence
shows the working directory when not at project root. A trailing `# exit
<code>, <dur>s` comment appears when the command failed or took over one second
(its absence means exit 0, fast):

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

All narration is delivered through a single path: the `attend listen` background
command. When you run `attend listen` and narration is pending, the PreToolUse
hook delivers the content and approves the command in one round trip — so the
narration arrives and a new listener starts simultaneously. This is why narration
only ever arrives when you run `attend listen`, never on other tool calls.

## After responding to narration

The listener is already running — it was started in the same round trip that
delivered the narration. It will wake you when the next narration arrives. Do
not restart it.

## When to restart the receiver

Internally note the task ID each time you run `attend listen` (it appears in
the tool result as "Command running in background with ID: …"). This is your
**current listener ID**. Never print or mention it to the user.

There are exactly two situations where you should run `attend listen`:

1. A `<task-notification>` arrives for an `attend listen` task — but **only if
   its task ID matches your current listener ID**. If the ID does not match,
   the notification is stale (an older listener) — ignore it. Do not read the
   task output file; it has no useful content.
2. You are told "narration is ready." This means narration arrived while you
   were using other tools. Run `attend listen` to receive it.

In all other situations, do **not** restart the receiver:

- After responding to narration — the listener is already running. Update your
  current listener ID to the task ID from this call's tool result.
- When told the listener is "already active" — one is running and will wake you.

## Edge cases

If narration contains only cursor/selection movements with no spoken words,
restart the listener without any acknowledgement. These are incidental editor
movements, not intentional messages.

Editor context is scoped to your current working directory. If the user
references files you can't see, they may have navigated outside it — suggest
adding paths to `include_dirs` in `.attend/config.toml` (or
`~/.config/attend/config.toml`).
