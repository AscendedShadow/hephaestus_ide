# IDE-024 — Manage multiple terminal sessions and configurable shell profiles

**Priority:** P2 · **Area:** Terminal · **Size:** M · **Dependencies:** None

## User outcome

Keep a server running while using another terminal, and choose the shell/environment needed by a project.

## Repository evidence

- [`shell.rs:249-252`](../../crates/desktop/src/shell.rs#L249-L252): the shell owns one interactive terminal and one separate run terminal.
- [`shell.rs:3695-3787`](../../crates/desktop/src/shell.rs#L3695-L3787): the Terminal tab displays that one interactive entity, with no session selector.
- [`terminal/lib.rs:22-28`](../../crates/terminal/src/lib.rs#L22-L28): PTY options already support shell arguments, environment and working directory.
- [`settings.rs:24-31`](../../crates/desktop/src/settings.rs#L24-L31): terminal profiles/default-shell settings are absent.

## Acceptance criteria

- [ ] Create, select, rename and close independent interactive terminal sessions with visible process/exit state.
- [ ] Configure named shell profiles with program, args, environment and initial working-directory policy; choose a default profile.
- [ ] Opening a terminal from an explorer directory starts it there, and users can create a terminal in the workspace directory from commands.
- [ ] Output, selection, focus and resize state remain isolated per session; closing one terminal leaves other processes intact.
- [ ] Closing sessions and switching workspaces have a defined process-lifecycle policy; profile failures identify the failing executable/configuration.

## Verification

Run different long-lived commands in two shells, switch sessions, resize, close one and restart an exited session. Confirm independent output/env/directories and no orphaned PTYs after window close.
