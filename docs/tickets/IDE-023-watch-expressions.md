# IDE-023 — Add persistent debugger watch expressions

**Priority:** P2 · **Area:** Debugger · **Size:** M · **Dependencies:** None

## User outcome

Monitor selected expressions automatically at each stop without retyping them in the debug console.

## Repository evidence

- [`debugger.rs:357-379`](../../crates/desktop/src/debugger.rs#L357-L379): debugger state contains scopes, variables, console and memory views, without a watch collection.
- [`debugger.rs:933-967`](../../crates/desktop/src/debugger.rs#L933-L967): console expression evaluation is one-shot.
- [`debugger.rs:969-1025`](../../crates/desktop/src/debugger.rs#L969-L1025): `watch` evaluation context is used to resolve memory addresses, not to maintain a watch list.
- [`dap/lib.rs:289-300`](../../crates/dap/src/lib.rs#L289-L300): a reusable evaluate request already exists.

## Acceptance criteria

- [ ] Add/edit/remove/reorder watch expressions in a debugger view and persist expressions per workspace.
- [ ] Evaluate watches in the selected stack frame on every stop and frame change using the DAP watch context.
- [ ] Show value, type and per-expression errors, and expand structured values using existing variable-child loading.
- [ ] Mark values stale while running; do not let a response from a previous stop/frame overwrite current values.
- [ ] Restarting a debug session preserves expressions but refreshes runtime references and values.

## Verification

Watch a scalar, a structured value and an invalid expression. Step and change frames with delayed responses, then restart; verify correct context, stale-state handling and expression persistence.
