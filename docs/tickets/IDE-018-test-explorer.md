# IDE-018 — Add test discovery, execution and a results explorer

**Priority:** P1 · **Area:** Testing · **Size:** L · **Dependencies:** None

## User outcome

Run a single test or suite and navigate failures directly from the IDE.

## Repository evidence

- [`shell.rs:88-106`](../../crates/desktop/src/shell.rs#L88-L106): tool/picker modes cover terminal, run, debug and tasks, without a test UI.
- [`settings.rs:33-41`](../../crates/desktop/src/settings.rs#L33-L41) and [`shell.rs:883-929`](../../crates/desktop/src/shell.rs#L883-L929): tests can be launched only as generic commands, without structured test identity/results.
- [`ide-core/lib.rs`](../../crates/ide-core/src/lib.rs): no test discovery/execution model is exposed.

## Acceptance criteria

- [ ] Define a provider-neutral model for test identity, hierarchy, source locations and execution state; ship an initial Cargo/Rust provider.
- [ ] Discover unit/integration tests by package and target, refresh after relevant changes, and represent discovery failures separately from failed tests.
- [ ] Run one test, a selected suite or all tests; support cancellation and rerun-failed without manually composing terminal commands.
- [ ] Display queued/running/passed/failed/skipped states, output and duration, with navigable source/failure locations.
- [ ] Preserve results while editing and distinguish identical test names in different packages/targets; run arguments use exact identities where the runner supports them.

## Verification

Use a small Cargo workspace with duplicate test names, an ignored test, a panic, an integration target and a compile failure. Verify single-test filtering, result attribution, cancellation and rerun-failed.
