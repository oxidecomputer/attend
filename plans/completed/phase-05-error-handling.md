# Phase 5: Error Handling Audit

**Dependencies**: Phase 3 (modules settled, changes are localized).
**Effort**: Medium | **Risk**: Low-Medium

---

## 5.1 `resolve_bin_cmd` — stop over-recovering

- Dev mode: use `current_exe()`, done (tightened during Camino migration — now errors on non-UTF-8)
- Release mode: `which` must succeed or return error — if we can't find the binary, neither can the agent
- Remove the remaining fallback chain that silently papers over missing binaries

## 5.2 `receive.rs` — remove legacy no-session fallback

- The `None =>` branch that tries `narration.json` should be removed
- No session ID = error, not a guess
- Fix stale help text: `"use --session"` -> reference `/attend`

## 5.3 `eprintln` vs `println` audit in receive.rs

- Agent reads stdout only; stderr goes nowhere in background tasks
- Every message intended for the agent must go to stdout
- `eprintln` reserved for debug/human-facing messages only

## 5.4 Systematic `let _ =` audit

- Review every `let _ =` across the codebase
- For each: is the error genuinely ignorable, or are we hiding a bug?
- Convert to proper error handling or add explicit `// Intentionally ignored: <reason>` comments

## 5.5 Lock file consistency

- `receive.rs` rolls its own lock with `O_CREAT | O_EXCL` while `record.rs` uses the `lockfile` crate
- Unify: either use `lockfile` everywhere, or find a PID-aware lock crate for both
- Investigate the 30-minute stale lock bug — does `lockfile::Lockfile` Drop run on SIGTERM/SIGKILL? Add signal handler if needed (`signal-hook` is already a dep)

## 5.6 `auto_upgrade_hooks` — rate-limit or relocate

- Currently runs on every hook invocation
- Consider: only on explicit user actions (`attend agent install`), or rate-limit (once per hour/session)
- At minimum, don't let upgrade failures block the hook response

---

## Verification

- `grep -rn 'let _ =' src/` — every hit has a `// Intentionally ignored:` comment or has been converted to proper error handling
- `grep -rn 'eprintln!' src/narrate/receive.rs` — zero hits (or each is justified as human-only debug output)
- `grep -rn 'unwrap_or_default()' src/` — each hit reviewed and justified
- `grep -rn '"--session"' src/` — zero hits (stale help text removed)
- Manual test: run `attend listen` without a session -> get a clear error, not a silent fallback
