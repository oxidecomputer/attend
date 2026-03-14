# Phase 2: Type Safety & Config Simplification

**Dependencies**: Phase 1 (camino dep available, constants extracted).
**Effort**: Medium | **Risk**: Low

---

## 2.1 Derive `serde::Deserialize` on `Engine` enum

Add `#[derive(serde::Deserialize)]` with `#[serde(rename_all = "lowercase")]` to `Engine`. This enables direct deserialization from TOML.

## 2.2 Eliminate `RawConfig`

With Engine deserializable, `Config` can derive `Deserialize` directly. Remove `RawConfig`, remove `parse_engine()`. Single struct.

## 2.3 Add `Config::merge` method

Extract inline merge logic from `Config::load()` into a `merge(&mut self, other: Config)` method. `load()` becomes: collect all config files -> deserialize each -> fold with `merge`.

## 2.4 Camino migration

Replace `PathBuf` / `Path` with `Utf8PathBuf` / `Utf8Path` throughout the codebase. Eliminates all `to_string_lossy()` and `to_str().unwrap_or_default()`. Non-UTF-8 paths fail at system boundary.
- Start with `state.rs` and `config.rs` (most path-heavy)
- Then `narrate/` modules
- Then `editor/`
- Then `view/`, `watch.rs`, `json.rs`
- Consolidate duplicate `relativize` functions (`state/resolve.rs` and `receive.rs`) into one shared utility

## 2.5 Introduce `SessionId` newtype

- `SessionId(String)` — replace `Option<String>` threading through hook/receive/record
- Update all function signatures, enabling the compiler to catch misuse
- ~~`WallClock(String)`~~ — dropped; chrono replaces manual timestamp strings in Phase 4.2
- ~~`ModelPath(Utf8PathBuf)`~~ — dropped; no distinct invariants beyond `Utf8PathBuf`

---

## Verification

Beyond the general gates:
- `grep -rn 'to_string_lossy' src/` returns zero hits
- `grep -rn 'to_str().unwrap_or_default()' src/` returns zero hits
- `grep -rn 'RawConfig' src/` returns zero hits
- `grep -rn 'parse_engine' src/` returns zero hits
- All existing config tests still pass (semantic equivalence with the old two-struct approach)
