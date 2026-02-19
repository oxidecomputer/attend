Activate dictation mode: run `{bin_cmd} dictate receive --wait` in the
background (Bash with `run_in_background: true`, `description: "💬"`). Do nothing
else.

Dictation input arrives through two paths:

- **Stop hook** (non-blocking): delivers pending dictation when you stop.
  No action needed — the hook handles this automatically.
- **Background receiver** (blocking): polls until dictation arrives, then
  prints it and exits. When this background task completes, immediately
  start a new one so you are always listening for the next dictation.

Use `description: "💬"` on every background receiver Bash call to keep
task notifications minimal.

Dictation arrives wrapped in `<dictation>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at and diff
blocks showing what code they changed. Treat it as the user's message — respond
to what they said and asked.
