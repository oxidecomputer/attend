# Development

## Building and testing

```bash
cargo fmt
cargo clippy
cargo nextest run
cargo build --release
```

All three gates (fmt, clippy, test) should be clean before every commit.

Use `cargo nextest run` rather than `cargo test` — it shows per-test timing,
which makes slow tests easy to spot. Because we use a mock clock in test mode,
tests should never sleep on wall-clock time, and slow tests are almost certainly
a bug.

### Architectural patterns

- **Trait-based backends.** Editors, agents, shells, and browsers each
  implement a trait on a zero-sized struct. Backends are registered in a
  static slice and the CLI wires them up automatically.

- **Filesystem-based IPC.** The CLI and daemon communicate through atomic
  command/status files, not persistent connections.

- **Pure decision logic.** The hook decision table (`hook/decision.rs`) is a
  pure function from session state to action, with no side effects.

- **Time abstraction.** All code uses a `Clock` trait. Production uses
  `RealClock`; tests use `MockClock` with deterministic, explicitly-advanced
  time. This lets us do a kind of "low-rent
  [Antithesis](https://antithesis.com/)" to semi-deterministically test
  scenarios including all the IPC.

See [How it works](how-it-works.md) for a higher-level architecture overview.

## Test architecture

Tests are organized by scope:

- **Unit tests** live in `<module>/tests.rs` sibling files (not inline
  `mod tests` blocks). Each test has a doc comment stating the invariant.

- **Integration tests** live in `tests/`. The test harness (`crates/test-harness`)
  provides E2E testing by spawning real processes and driving them via CLI.

- **Property tests** use `proptest` for state-invariant testing. Regression
  files in `proptest-regressions/` are committed to the repo to ensure
  discovered failures reproduce deterministically.

### The test harness

The test harness (`crates/test-harness`) enables deterministic E2E testing:

- Spawns real `attend` processes with `ATTEND_TEST_MODE=1`
- Injects events (audio, editor state, time) via a Unix socket protocol
- Uses `MockClock` across all processes for lockstep time advancement
- Settlement protocol ensures all threads complete their work before time
  advances
- No polling or real-time waits — condvar-based coordination throughout

## Dev installation

Install `attend` hooks pointed at your local fork:

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
