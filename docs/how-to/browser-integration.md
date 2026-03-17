# How to add browser integration

This guide shows you how to capture text selections from your browser and
deliver them as narration context alongside speech and editor state.

## Firefox

```bash
attend install --browser firefox
```

This installs a native messaging host manifest and opens the signed extension
for installation. After clicking "Add" in Firefox, the extension persists
across restarts.

## Chrome

```bash
attend install --browser chrome
```

This installs a native messaging host manifest and writes an unpacked extension
to a persistent directory. Load it manually:

1. Open `chrome://extensions`.
2. Enable Developer mode.
3. Click "Load unpacked" and select the directory printed by the install command.

Chrome unpacked extensions persist across browser restarts. If `attend` is
updated and the extension files change, open `chrome://extensions` and click the
reload button on the attend extension to pick up the new version.

## What you get

When narration is active, text you select in the browser is captured with the
page URL and title, and delivered to your agent alongside speech and editor
context. See [narration format](../reference/narration-format.md#browser-selections) for how
browser selections appear in narration.
