The narration loop must be **completely silent**. Never produce output about
listener state — no "listening", "restarting", "standing by", or any other
status commentary. The user knows they are in narration mode. The only
visible output should be your responses to actual narration content.

## How narration arrives

Narration arrives wrapped in `<narration>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at and diff
blocks showing what code they changed. Treat it as the user's message — respond
to what they said and asked.

Narration is delivered through two independent paths:

- **Hook delivery**: pending narration is injected automatically when you stop
  or between tool calls. This happens regardless of receiver state.
- **Background receiver**: a background `listen` command that polls until
  narration is pending, then exits. It exists solely as a wake-up mechanism —
  when it exits, the resulting tool activity triggers hook delivery.

These paths are independent. Hooks can deliver narration whether or not a
receiver is running.

## When to restart the receiver

Restart the receiver when either of these occurs:

1. You receive a `<task-notification>` indicating the background receiver has
   exited.
2. You are told "narration is ready" — this means narration arrived after your
   last tool call. Run `attend listen` to receive it; the narration will be
   delivered when the receiver starts.

Do **not** restart the receiver:
- After responding to hook-delivered narration — the receiver may still be
  running. Hook delivery and receiver lifecycle are independent.
- When told the listener is "already active for this session" — a working
  listener exists and will wake you up.

## Edge cases

If narration contains only cursor/selection movements with no spoken words, just
restart the listener without any acknowledgement. These are incidental editor
movements, not intentional messages.

Editor context is scoped to your current working directory. If the user
references files you can't see, they may have navigated outside it — suggest
adding paths to `include_dirs` in `.attend/config.toml` (or
`~/.config/attend/config.toml`).
