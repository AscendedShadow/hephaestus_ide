# IDE-026 — Create and switch Git branches from the IDE

**Priority:** P2 · **Area:** Source control · **Size:** M · **Dependencies:** None

## User outcome

Change development branches and create a feature branch without opening an external Git client.

## Repository evidence

- [`git.rs:74-98`](../../crates/ide-core/src/git.rs#L74-L98): current branch/upstream/ahead/behind metadata is available.
- [`git.rs:347-615`](../../crates/ide-core/src/git.rs#L347-L615): repository operations cover init/clone/status/diff/staging/commit/remotes, without branch listing/creation/switching.
- [`git_panel.rs:35`](../../crates/desktop/src/git_panel.rs#L35): Git actions cover commit, stage-all and remote synchronization.

## Acceptance criteria

- [ ] A searchable branch picker lists local/remote branches, current selection and useful upstream status.
- [ ] Users can create a branch from a selected ref, switch local branches, and check out a remote branch with explicit tracking behavior.
- [ ] Invalid names, conflicting local changes and Git failures are surfaced with actionable messages; dirty editor buffers receive an explicit disposition before switching.
- [ ] Successful switching refreshes open documents, explorer/search state, diffs and Git status, preserving appropriate cursor/view state.
- [ ] Branch operations run asynchronously and cannot be duplicated while one is pending.

## Verification

Use a temporary repository and bare remote to test local creation, remote tracking and a switch blocked by worktree changes. Include a dirty open buffer and verify no content is silently lost.
