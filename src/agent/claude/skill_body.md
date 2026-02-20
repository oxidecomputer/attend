Activate narration mode: run `{bin_cmd} listen` in the background (Bash with
`run_in_background: true`, `description: "💬"`). Do not acknowledge activation
or produce any other output.

IMPORTANT: Use the exact command `{bin_cmd} listen` as written — it has been
whitelisted during installation. Do not expand or rewrite the path.

The narration loop must be **completely silent**. Never produce output about
listener state — no "listening", "restarting", "standing by", or any other
status commentary. The user knows they are in narration mode. The only
visible output should be your responses to actual narration content.

Narration input arrives through two paths:

- **Stop hook** (non-blocking): delivers pending narration when you stop.
  No action needed — the hook handles this automatically.
- **Background receiver** (blocking): polls until narration arrives, then
  prints it and exits. When this background task completes, immediately start a
  new one so you are always listening for the next narration. If the receiver
  exits without producing `<narration>` tags and without a session-transfer
  message, just restart it — this is a transient condition, not a permanent
  error.

Use `description: "💬"` on every background receiver Bash call to keep task
notifications minimal.

Narration arrives wrapped in `<narration>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at and diff
blocks showing what code they changed. Treat it as the user's message — respond
to what they said and asked.

If narration contains only cursor/selection movements with no spoken words,
just restart the listener. These are incidental editor movements, not
intentional messages.

Editor context is scoped to your current working directory. If the user
references files you can't see, they may have navigated outside it — suggest
adding paths to `include_dirs` in `.attend/config.toml` (or
`~/.config/attend/config.toml`).
