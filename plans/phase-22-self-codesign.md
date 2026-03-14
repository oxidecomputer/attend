# Phase 22: Self-Codesign on First Run

## Problem

When `attend` is reinstalled via `cargo install`, the new binary has a different
code hash. macOS TCC ties accessibility permissions to the binary's code
signature identity. After reinstall:

1. The old entry remains in System Preferences > Accessibility, but is invalid.
2. Accessibility API calls fail silently.
3. iTerm2's auto-copy means clipboard capture grabs selections instead of the
   accessibility-based external capture.
4. Result: selections arrive as plain blockquotes (no app attribution): silent
   degradation with no diagnostic.

## Solution

When the recording daemon launches (macOS only), check whether the binary is
codesigned with a stable identity (`attend-dev`). If not, create the certificate
(if needed), sign the binary, and re-exec so TCC sees the signed binary for
this invocation.

TCC grants persist across reinstalls because the signing identity is stable.

## Design

### Where it runs

The codesign check runs in `record::daemon()`, not in `main()`. Only the
daemon needs accessibility and microphone permissions; other subcommands
(`glance`, `look`, `listen`, `hook`, etc.) don't. This keeps the common
path fast and avoids surprising users who run `attend look`.

The `meditate` (state cache daemon) command reads editor state via the Zed
JSON-RPC API, not the accessibility API, so it does not need codesigning.

### Startup flow

```
record::daemon()
  └─ codesign::ensure_signed()?   // #[cfg(target_os = "macos")], no-op on Linux
       ├─ is_signed_with("attend-dev")?  → return Ok(())
       ├─ has_identity("attend-dev")?    → skip cert creation
       │    └─ no → create_identity("attend-dev")
       ├─ sign_self("attend-dev")?
       └─ re_exec()?                     // exec new binary with same args
```

### Step 1: Check current signature

Shell out to `codesign -dvvv <current_exe>` and parse output for
`Authority=attend-dev`. If found, return early.

If the command fails (unsigned binary) or the authority doesn't match, proceed.

### Step 2: Check for existing certificate

Shell out to `security find-identity -p codesigning -v` and look for
`"attend-dev"` in output. If found, skip to step 3.

### Step 3: Create self-signed certificate

1. Use `rcgen` to generate a self-signed X.509 certificate:
   - Subject CN: `attend-dev`
   - Extended Key Usage: Code Signing (OID `1.3.6.1.5.5.7.3.3`)
   - Validity: 10 years (local dev cert, doesn't matter much)
2. Package cert + private key as PKCS#12 (`.p12`).
3. Write `.p12` to a tempfile.
4. Import into login keychain:
   ```
   security import <tempfile.p12> \
     -k ~/Library/Keychains/login.keychain-db \
     -P <password> \
     -T /usr/bin/codesign
   ```
   `-T /usr/bin/codesign` pre-authorizes codesign to access the key
   without prompting on subsequent uses.
5. Clean up tempfile.

**Permissions:** No sudo required. Login keychain is user-owned and typically
unlocked during a session. May trigger a one-time macOS keychain GUI dialog
on first certificate import.

### Step 4: Sign the binary

A running binary cannot be modified in place (`ETXTBSY`). Use the
unlink-write-sign pattern:

1. `let exe = std::env::current_exe()?`
2. `let bytes = std::fs::read(&exe)?` — read binary into memory.
3. `std::fs::remove_file(&exe)?` — unlink path (process keeps old inode).
4. `std::fs::write(&exe, &bytes)?` — write to same path (new inode).
5. Restore executable permission (`std::fs::set_permissions`).
6. `codesign -fs "attend-dev" <exe>` — sign the new file.

### Step 5: Re-exec

The current process is still running from the old (deleted) unsigned inode.
Re-exec so TCC sees the signed binary for this invocation:

```rust
use std::os::unix::process::CommandExt;
let err = std::process::Command::new(std::env::current_exe()?)
    .args(std::env::args_os().skip(1))
    .exec();
// exec() only returns on error
Err(err.into())
```

The re-exec'd process hits step 1, sees it's already signed, and proceeds
normally into the daemon main loop.

## Crate layout

New workspace member: `crates/macos-codesign/`

```
crates/macos-codesign/
├── Cargo.toml
└── src/
    └── lib.rs
```

**Dependencies:**
- `rcgen` — certificate generation (code signing EKU)
- `p12` (or equivalent) — PKCS#12 bundling
- `tempfile` — temporary `.p12` file
- `anyhow` — error handling

**Workspace Cargo.toml addition:**
```toml
[workspace]
members = [
    ".", "crates/mock-clock", "crates/macos-disclaim",
    "crates/macos-codesign", "crates/test-harness", "tools/xtask",
]
```

**Main binary dependency** (platform-gated):
```toml
[target.'cfg(target_os = "macos")'.dependencies]
macos-codesign = { path = "crates/macos-codesign" }
```

## Public API

```rust
/// Ensure the current binary is codesigned with a stable identity.
///
/// On first run after `cargo install`, creates a self-signed certificate
/// (if needed), signs the binary, and re-execs. On subsequent runs, this
/// is a fast no-op (one `codesign -dvvv` call).
///
/// # Returns
///
/// Returns `Ok(())` on the fast path (already signed).
/// On the sign+re-exec path, this function does not return (it execs).
///
/// # Errors
///
/// Returns an error if signing fails. The caller should log the warning
/// and continue: accessibility capture may not work, but the daemon
/// remains functional.
pub fn ensure_signed() -> anyhow::Result<()>;
```

## Integration in record.rs

```rust
pub fn daemon(clock: Arc<dyn Clock>) -> anyhow::Result<()> {
    let _ = nix::unistd::setsid();

    #[cfg(target_os = "macos")]
    if !crate::test_mode::is_active() {
        if let Err(e) = macos_codesign::ensure_signed() {
            tracing::warn!("self-codesign failed: {e:#}");
        }
    }

    // ... existing daemon code ...
}
```

Skipped in test mode since tests run unsigned binaries and don't need
accessibility.

## Edge cases

- **Test mode:** Skipped (`test_mode::is_active()` guard). Tests exercise
  codesign logic through a mock/stub layer (see Testing section).
- **Non-writable install path:** If `current_exe()` is in a read-only location
  (e.g., `/usr/local/bin` owned by root), the unlink will fail. Log warning,
  continue unsigned. `cargo install` defaults to `~/.cargo/bin/` which is
  user-writable.
- **Keychain locked:** `security import` may fail or prompt. Log warning,
  continue.
- **Certificate already exists but key is inaccessible:** `codesign -fs` will
  fail. Log warning, continue.
- **Re-exec loop prevention:** The re-exec'd process checks `codesign -dvvv`
  and sees it's signed, taking the fast path. No loop possible unless
  signing silently produces a bad signature, in which case the check fails
  again and re-exec occurs. Guard with an env var
  (`ATTEND_CODESIGN_ATTEMPTED=1`) to break any theoretical loop: if set,
  skip codesign entirely.
- **Linux:** All of this is `#[cfg(target_os = "macos")]`. Linux is unaffected.

## Diagnostics

Structured tracing at appropriate levels:

- `tracing::debug!("already signed with attend-dev identity")` — fast path
- `tracing::info!("created attend-dev certificate in login keychain")` — cert
- `tracing::info!("signed binary, re-execing")` — sign + re-exec
- `tracing::warn!("self-codesign failed: {e:#}")` — any failure

## Testing

The codesign logic shells out to `codesign` and `security`, which makes
direct e2e testing tricky (tests run unsigned, modifying the test binary
would affect other parallel tests, keychain operations are side-effectful).

### Strategy: trait-based command abstraction

Extract the shell commands behind a trait so the real implementation shells
out and a test implementation uses fakes:

```rust
pub trait CodesignBackend {
    /// Check if `path` is signed with `identity`. Returns the signing
    /// authority if signed, or None if unsigned/different identity.
    fn signing_authority(&self, path: &Path) -> anyhow::Result<Option<String>>;

    /// Check if a codesigning identity exists in the keychain.
    fn has_identity(&self, name: &str) -> anyhow::Result<bool>;

    /// Create a self-signed codesigning certificate and import it.
    fn create_identity(&self, name: &str) -> anyhow::Result<()>;

    /// Sign the binary at `path` with the given identity.
    fn sign(&self, path: &Path, identity: &str) -> anyhow::Result<()>;
}
```

### Unit tests (in `crates/macos-codesign`)

Test the `ensure_signed` state machine with a `FakeBackend`:

1. **Already signed:** `signing_authority` returns `Some("attend-dev")`.
   Verify: returns `Ok(())`, no other methods called.

2. **Unsigned, cert exists:** `signing_authority` returns `None`,
   `has_identity` returns `true`. Verify: `sign` called, re-exec
   attempted (detectable via the env var or a callback).

3. **Unsigned, no cert:** `signing_authority` returns `None`,
   `has_identity` returns `false`. Verify: `create_identity` called,
   then `sign`, then re-exec.

4. **Sign failure:** `sign` returns `Err`. Verify: error propagated,
   no re-exec.

5. **Cert creation failure:** `create_identity` returns `Err`. Verify:
   error propagated, `sign` not called.

6. **Loop guard:** `ATTEND_CODESIGN_ATTEMPTED` set. Verify: returns
   `Ok(())` immediately, no methods called.

### E2e tests (in `tests/e2e.rs`)

The e2e harness spawns real `attend` processes with `ATTEND_TEST_MODE=1`.
Since test mode skips codesign, e2e tests can't directly exercise the
signing flow. Instead, test the observable contract:

1. **Daemon starts when codesign is skipped (test mode):** Already
   covered by existing e2e tests — the daemon launches and records
   without codesigning.

2. **Env var loop guard works end-to-end:** Spawn daemon with
   `ATTEND_CODESIGN_ATTEMPTED=1` set (via harness env override).
   Verify it starts normally without hanging or re-exec looping.
   This requires adding env-override support to the harness's
   `spawn_command`.

3. **Graceful degradation:** If the harness could set a flag causing
   `ensure_signed` to fail (e.g., a test-only error injection env var),
   verify the daemon still starts and records, just with a warning in
   stderr. This validates the non-fatal error handling path.

### Manual testing checklist

These require a real macOS system and cannot be automated:

- [ ] Fresh install (`cargo install --path .`): certificate created,
      binary signed, `codesign -dvvv` shows `attend-dev`.
- [ ] Reinstall: no cert creation, binary re-signed, accessibility
      permission persists without re-granting.
- [ ] `attend narrate toggle`: daemon re-execs after signing, narration
      works, external selections have app attribution.
- [ ] Non-writable path: `sudo cp target/debug/attend /usr/local/bin/`,
      run from there — warning logged, daemon still functions.

## Task ordering

1. Create `crates/macos-codesign` with `Cargo.toml` and dependencies.
2. Define `CodesignBackend` trait and `RealBackend` (shell-out impl).
3. Implement `ensure_signed()` state machine using the trait.
4. Add `FakeBackend` and unit tests for the state machine.
5. Integrate into `record::daemon()` with test-mode guard.
6. Add env-override support to test harness `spawn_command`.
7. Add e2e tests for loop guard and graceful degradation.
8. Manual testing on macOS.
