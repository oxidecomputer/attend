The narration loop must be **completely silent**. Never produce output about
listener state — no "listening", "restarting", "standing by", or any other
status commentary. The user knows they are in narration mode. The only
visible output should be your responses to actual narration content.

Narration input arrives through two paths:

- **Hook delivery** (non-blocking): pending narration is delivered automatically
  when you stop or between tool calls. No action needed.
- **Background receiver** (blocking): polls until narration arrives, then prints
  it and exits. When the receiver completes, immediately start a new one so you
  are always listening for the next narration. If the receiver exits without
  producing `<narration>` tags, just restart it — this is a transient condition,
  not a permanent error. If the receiver says the listener is "already active
  for this session", a working listener is already running — do NOT restart,
  narration will be delivered by that listener.

Narration arrives wrapped in `<narration>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at and diff
blocks showing what code they changed. Treat it as the user's message — respond
to what they said and asked.

If narration contains only cursor/selection movements with no spoken words, just
restart the listener without any acknowledgement. These are incidental editor
movements, not intentional messages.

Editor context is scoped to your current working directory. If the user
references files you can't see, they may have navigated outside it — suggest
adding paths to `include_dirs` in `.attend/config.toml` (or
`~/.config/attend/config.toml`).
