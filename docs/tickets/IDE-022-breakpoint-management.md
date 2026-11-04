# IDE-022 — Persist breakpoints and add conditions, logpoints and exception filters

**Priority:** P2 · **Area:** Debugger · **Size:** L · **Dependencies:** None

## User outcome

Keep useful breakpoints across sessions and stop only when a relevant condition or exception occurs.

## Repository evidence

- [`debugger.rs:357-360`](../../crates/desktop/src/debugger.rs#L357-L360) and [`debugger.rs:465-513`](../../crates/desktop/src/debugger.rs#L465-L513): breakpoints are in-memory sets of line numbers, with toggle/remove behavior.
- [`dap/lib.rs:240-257`](../../crates/dap/src/lib.rs#L240-L257): `setBreakpoints` serializes only the line field.
- [`debugger.rs:715-737`](../../crates/desktop/src/debugger.rs#L715-L737): exception configuration always sends an empty filter list.
- [`session.rs:8-16`](../../crates/desktop/src/session.rs#L8-L16): session persistence has no breakpoint state.

## Acceptance criteria

- [ ] Persist workspace-relative breakpoint identities, enabled state, condition, hit condition and log message; enable/disable without deleting.
- [ ] Send supported DAP condition/hit/log fields and expose adapter-provided exception breakpoint filters.
- [ ] Retain adapter capabilities and explain unsupported breakpoint options instead of silently ignoring them.
- [ ] Track breakpoint positions through source edits/renames, reconcile adapter-adjusted locations and handle breakpoint verification/update events.
- [ ] Restore breakpoint configuration on IDE restart and synchronize changes during an active session without duplicating entries.

## Verification

Mock conditional/hit/logpoint requests and adapter relocation/events. Restart the IDE, insert lines above a breakpoint, rename its file and verify persisted state and actual stop location remain consistent.
