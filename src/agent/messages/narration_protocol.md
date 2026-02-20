Treat narration like any normal conversation — respond naturally, use tools if
the task calls for it, and stop when you're done.

Never produce output about listener state — no "listening", "restarting",
"standing by", or any other status commentary. The user is speaking to you by
voice while looking at code. They can see your responses; they don't need a
play-by-play about the delivery mechanism. The only visible output should be
your responses to what they actually said.

## How narration arrives

Narration arrives wrapped in `<narration>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at and diff
blocks showing what code they changed. Treat it as the user's message — respond
to what they said and asked.

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

There are exactly two situations where you should run `attend listen`:

1. A `<task-notification>` says the background receiver exited — but only if
   you haven't started a newer listener since then. Narration delivery starts
   a replacement listener automatically, so notifications for older listeners
   are stale. Do not read the task output file; it has no useful content.
2. You are told "narration is ready." This means narration arrived while you
   were using other tools. Run `attend listen` to receive it.

In all other situations, do **not** restart the receiver:

- After responding to narration — the listener is already running.
- When told the listener is "already active" — one is running and will wake you.

## Edge cases

If narration contains only cursor/selection movements with no spoken words,
restart the listener without any acknowledgement. These are incidental editor
movements, not intentional messages.

Editor context is scoped to your current working directory. If the user
references files you can't see, they may have navigated outside it — suggest
adding paths to `include_dirs` in `.attend/config.toml` (or
`~/.config/attend/config.toml`).
