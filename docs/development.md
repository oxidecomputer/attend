# Development

## Building and testing

```bash
cargo fmt
cargo clippy
cargo test
cargo build --release
```

All three gates (fmt, clippy, test) should be clean before every commit.

## Dev installation

Globally install `attend` hooks pointed at your local fork like this:

```bash
cargo run -- install --dev --agent <agent> --editor <editor>
```

The `--dev` flag points the installed hooks at your local build instead of
the release binary, so changes take effect immediately.

## xtasks

Code generation and release tasks live in `tools/xtask/`. Run them with:

```bash
cargo xtask <command>
```

| Command | Purpose |
|---------|---------|
| `gen-gfm-languages` | Regenerate `src/view/gfm_languages.rs` from GitHub Linguist's `languages.yml` |
| `sign-extension` | Sign the Firefox extension as an unlisted AMO add-on |

### `gen-gfm-languages`

Fetches the canonical list of language names and aliases from GitHub
Linguist, then generates a sorted `&[&str]` constant used for GFM
fenced-code-block syntax detection.

### `sign-extension`

Signs the Firefox extension via the AMO (addons.mozilla.org) API. Requires
`web-ext` on PATH and two environment variables:

- `AMO_JWT_ISSUER` — API key (JWT issuer) from addons.mozilla.org
- `AMO_JWT_SECRET` — API secret from addons.mozilla.org

Produces `extension/attend.xpi`. Rebuild attend after signing to embed the
`.xpi` in the binary.
