# IDE-019 — Add task dependencies, build defaults and pre-debug tasks

**Priority:** P1 · **Area:** Build/run · **Size:** L · **Dependencies:** IDE-016

## User outcome

Build and run/debug a project through repeatable task sequences, including long-running background services.

## Repository evidence

- [`settings.rs:33-54`](../../crates/desktop/src/settings.rs#L33-L54): run configurations are independent program/args/env records; debugger configuration has no pre-launch task.
- [`shell.rs:883-974`](../../crates/desktop/src/shell.rs#L883-L974): task launch and debugger launch are separate paths.
- [`terminal_view.rs:112-118`](../../crates/desktop/src/terminal_view.rs#L112-L118): a running task prevents another command from starting in the single task terminal.

## Acceptance criteria

- [ ] Tasks have stable IDs, default build/run roles, dependency references and a defined sequential/parallel dependency policy.
- [ ] Run/debug configurations can depend on a task; failed or cancelled prerequisites prevent the dependent launch.
- [ ] Support documented variables for workspace directory and active file, plus existing environment/working-directory fields.
- [ ] Background tasks can signal readiness independently of process exit; task state and output remain identifiable per execution.
- [ ] Detect dependency cycles/missing IDs, support rerun/cancel, and define cleanup for dependent process trees and workspace switches.

## Verification

Execute build-then-run, failed-build-then-debug, parallel prerequisites and a readiness-signalling service. Verify launch ordering, cancellation, cycle validation and process cleanup.
