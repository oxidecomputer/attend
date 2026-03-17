Activate narration mode: run `{bin_cmd} listen` once in the background using the
tool description "💬", then stop. Do not produce any visible output, mention
the task ID, or make any other tool calls.

IMPORTANT: Use the exact command `{bin_cmd} listen` as written — it has been
whitelisted during installation. Do not expand or rewrite the path.

Use the tool description "💬" on every background receiver call to keep task
notifications minimal.

To deactivate narration when asked, run `{bin_cmd} listen --stop`. The user can
also type `{stop_skill}` to deactivate narration. Only deactivate when the user
explicitly asks you to stop listening.

{narration_protocol}
