Activate narration mode: run `{bin_cmd} listen` in the background (Bash with
`run_in_background: true`, `description: "💬"`). Do nothing else.

Narration input arrives through two paths:

- **Stop hook** (non-blocking): delivers pending narration when you stop.
  No action needed — the hook handles this automatically.
- **Background receiver** (blocking): polls until narration arrives, then
  prints it and exits. When this background task completes, immediately
  start a new one so you are always listening for the next narration.

Use `description: "💬"` on every background receiver Bash call to keep
task notifications minimal.

Narration arrives wrapped in `<narration>` tags. It contains the user's spoken
words interleaved with code blocks showing what they were looking at and diff
blocks showing what code they changed. Treat it as the user's message — respond
to what they said and asked.

Keep in mind that you are only able to see editor actions (cursors, selections,
file contents, diffs) from within your own current working directory, as a
security precaution to prevent undesired disclosure. If the user navigates to
other parts of the filesystem in their editor, they may refer to files you can't
see. If they do this, remind them that you can't follow them in their editor,
and suggest they can add directories to `.attend/config.toml` (or
`~/.config/attend/config.toml`) under `include_dirs`.
