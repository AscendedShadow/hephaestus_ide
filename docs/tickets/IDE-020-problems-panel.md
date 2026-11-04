# IDE-020 — Add a persistent Problems panel and complete live build diagnostics

**Priority:** P1 · **Area:** Diagnostics/build · **Size:** L · **Dependencies:** None

## User outcome

Track errors and warnings across a project while a build runs, without losing diagnostics that scroll out of the terminal viewport.

## Repository evidence

- [`shell.rs:652-695`](../../crates/desktop/src/shell.rs#L652-L695): inline diagnostics exist, but navigation entries reduce each diagnostic to a location and first message line.
- [`language_server.rs:188-193`](../../crates/desktop/src/language_server.rs#L188-L193): diagnostics are forwarded only for tracked open documents.
- [`shell.rs:420-431`](../../crates/desktop/src/shell.rs#L420-L431): build-location extraction waits for task exit.
- [`terminal_view.rs:164-185`](../../crates/desktop/src/terminal_view.rs#L164-L185) and [`output.rs:5-35`](../../crates/ide-core/src/output.rs#L5-L35): extraction uses terminal snapshot rows and a simple location parser, rather than a complete output stream.

## Acceptance criteria

- [ ] Provide a persistent, keyboard-navigable Problems panel grouped by file/source, preserving severity, code, full message and related locations.
- [ ] Filter by severity, source and active file, show counts, and add next/previous problem navigation.
- [ ] Capture the complete task output incrementally, independent of viewport/scrollback size, and parse diagnostics while the task runs.
- [ ] Preserve workspace diagnostics for unopened files when the server publishes them; clear/replace stale results by source and build execution.
- [ ] Support at least Rust/Cargo and conventional compiler location output, handling ANSI escapes, chunk boundaries and Windows paths.

## Verification

Stream a build with diagnostics before more than one screen of output, split messages across chunks, and keep the process running. Verify early errors appear immediately and remain navigable after exit and subsequent rebuild.
