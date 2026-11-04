# IDE-021 — Select named debug profiles and attach to running processes

**Priority:** P1 · **Area:** Debugger · **Size:** L · **Dependencies:** IDE-016

## User outcome

Choose among project debug targets and debug an already-running application.

## Repository evidence

- [`settings.rs:24-30`](../../crates/desktop/src/settings.rs#L24-L30) and [`settings.rs:43-54`](../../crates/desktop/src/settings.rs#L43-L54): settings allow one optional debugger configuration with launch-oriented fields.
- [`shell.rs:943-974`](../../crates/desktop/src/shell.rs#L943-L974): starting the debugger always selects that one configuration.
- [`debugger.rs:582-589`](../../crates/desktop/src/debugger.rs#L582-L589): initialization always issues the DAP `launch` request.
- [`dap/lib.rs:323-328`](../../crates/dap/src/lib.rs#L323-L328): disconnect always requests debuggee termination, which needs a different policy for attach sessions.

## Acceptance criteria

- [ ] Store/select named debug configurations per workspace, remember the selection and migrate the existing single-debugger configuration.
- [ ] Support explicit launch versus attach requests with adapter-specific arguments and useful validation.
- [ ] A local attach workflow selects or enters a process ID; generic adapter arguments permit other attach targets without hardcoding one debugger.
- [ ] Stop/disconnect behavior respects session type, preserving attached processes unless termination was explicitly chosen.
- [ ] Restart, working-directory/environment resolution and errors identify the selected profile; switching profiles does not leak adapter processes.

## Verification

Use a mock adapter to verify launch/attach arguments and disconnect flags. Smoke-test two launch profiles and attachment to a long-running local process, confirming detach leaves it alive.
