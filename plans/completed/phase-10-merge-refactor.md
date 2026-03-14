# Phase 10: merge.rs Deep Refactor

**Dependencies**: Phase 9 (comprehensive test suite in place FIRST — highest-risk change).
**Effort**: Large | **Risk**: High

---

## 10.1 Comprehensive test suite for merge.rs

- Prop tests over arbitrary event stream permutations
- Cover every edge case: empty streams, single events, all-words, all-snapshots, interleaved, rapid-fire cursor changes
- Snapshot tests for rendered markdown output
- This must be complete before touching the implementation

## 10.2 Single streaming pass rewrite

- Replace multi-pass (`compress_snapshots` -> `merge_adjacent` -> `merge_diffs`) with composed fold/unfold
- Each transformation phase defined as a separate, composable function
- Document each phase's contract explicitly (input invariants -> output invariants)
- Verify all existing tests still pass

## 10.3 Documentation

- Document the event stream format and each transformation's purpose
- Explain the merge semantics for diffs (net change across a period)
- Explain snapshot compression rules

---

## Verification

- All Phase 9 merge tests pass (the entire point of writing them first)
- Snapshot test output is byte-for-byte identical to pre-refactor (no behavioral change)
- `cargo bench` (if merge-related benchmarks exist) shows no regression
- Code review: each composable function has a clear doc comment stating input->output contract
- No multi-pass iteration over the event list — single pass confirmed by inspection
