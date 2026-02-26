# Adding a new browser

A browser backend installs a native messaging host manifest so the browser
extension can communicate with `attend`, and provides the extension itself.

## How browser capture works

Browser capture uses the native messaging protocol:

1. A content script in the browser extension detects text selection.
2. The extension calls `browser.runtime.sendNativeMessage("attend", { html, url, title })`.
3. The browser launches `attend browser-bridge` (via a wrapper script).
4. The bridge reads one JSON message from stdin, converts HTML to markdown
   (via `htmd`), and atomically writes a `BrowserSelection` event to the
   browser staging directory.
5. The recording daemon collects these events and merges them into the narration.

The bridge is a one-shot process (Firefox launches a new process per
message). The same extension source (`extension/`) is shared across browsers;
only the manifest format differs.

## 1. Create the module — `src/browser/<name>.rs`

Implement the `Browser` trait:

```rust
pub struct Name;

impl Browser for Name {
    fn name(&self) -> &'static str { "<name>" }

    fn install(&self, bin_cmd: &str) -> anyhow::Result<()> {
        // 1. Install a native messaging host manifest pointing at
        //    bin_cmd (the attend-browser-bridge wrapper script).
        //    Use the `native_messaging::install` crate.
        //    Host name: "attend".
        //
        // 2. Install or print instructions for loading the extension.
        //    Firefox: write the signed .xpi and open it.
        //    Chrome: write the unpacked extension directory.
        Ok(())
    }

    fn uninstall(&self) -> anyhow::Result<()> {
        // Remove the native messaging manifest and any extension files.
        Ok(())
    }
}
```

### `Browser` trait methods

| Method         | Required | Purpose                                          |
|----------------|----------|--------------------------------------------------|
| `name()`       | yes      | CLI name, e.g. `"firefox"`                       |
| `install()`    | yes      | Install native messaging manifest + extension    |
| `uninstall()`  | yes      | Remove manifest and extension files              |

## 2. Register the backend in `src/browser.rs`

Add the module and register it in the `BROWSERS` slice:

```rust
mod chrome;
mod firefox;
mod <name>;
```

```rust
pub const BROWSERS: &[&'static dyn Browser] = &[
    &chrome::Chrome,
    &firefox::Firefox,
    &<name>::Name,
];
```

The CLI (`install --browser <name>`, `uninstall --browser <name>`) is built
automatically from the registered backends.

## Implementation notes

- **Native messaging manifest**: Use the `native_messaging` crate's install
  function. The host name must be `"attend"` (matching the extension's
  `sendNativeMessage` call). The `bin_cmd` parameter is the path to the
  `attend-browser-bridge` wrapper script, not the main binary.
- **Extension source**: The shared extension files live in `extension/`
  (`content.js`, `background.js`). Each browser has its own `manifest.json`
  format. Firefox uses WebExtension manifest v2; Chrome uses manifest v3.
- **Extension distribution**: Firefox extensions can be signed via AMO
  (see `cargo xtask sign-extension`) and embedded as an `.xpi` at compile
  time. Chrome extensions are installed as an unpacked directory.
- **Idempotency**: `install()` must be safe to call repeatedly.

## Checklist

- [ ] `src/browser/<name>.rs` — `pub struct Name` + `impl Browser for Name`
- [ ] `src/browser.rs` — `mod <name>;` declaration
- [ ] `src/browser.rs` — add `&<name>::Name` to `BROWSERS`
- [ ] Extension `manifest.json` for the target browser (if format differs)
