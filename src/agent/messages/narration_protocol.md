Treat narration like any normal conversation — respond naturally, use tools if
the task calls for it, and stop when you're done.

Never produce visible output about listener state — no "listening",
"restarting", "standing by", task IDs, or any other status commentary. The
user is speaking to you by voice while looking at code. They can see your
responses; they don't need a play-by-play about the delivery mechanism. The
only visible output should be your responses to what they actually said.

## How narration arrives

Narration arrives wrapped in `<narration>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at, diff
blocks showing what code they changed, blockquotes showing text they selected
in external applications, and fenced code blocks showing shell commands they
ran. Treat it as the user's message — respond to what they said and asked.

External selections appear as blockquotes with a source annotation:

> [iTerm2: ~/src/attend] "error[E0308]: mismatched types"

The format is `> [AppName: WindowTitle] "selected text"`. These show text the
user highlighted in applications outside the editor (e.g. terminal output,
documentation in Safari). Treat them as context for what the user is talking
about, just like editor snapshots.

Shell commands appear as fenced code blocks tagged with the shell name:

````fish
# in subdir/
cargo test --lib  # exit 1, 3.2s
````

The shell tag (e.g. `fish`, `zsh`) identifies the shell. An optional `# in
<dir>/` comment shows the working directory when it differs from the project
root. A trailing `# exit <code>, <dur>s` comment appears when the command
failed or took more than one second — its absence means exit 0, fast. These
give you a timeline of what the user did in their terminal alongside what they
said and looked at.

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
