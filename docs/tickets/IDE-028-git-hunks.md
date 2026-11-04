# IDE-028 — Stage and unstage individual Git hunks

**Priority:** P2 · **Area:** Source control/diff · **Size:** L · **Dependencies:** None

## User outcome

Commit one logical change while leaving unrelated changes in the same file unstaged.

## Repository evidence

- [`git.rs:473-488`](../../crates/ide-core/src/git.rs#L473-L488): staging/unstaging operates on whole file paths.
- [`git_panel.rs:422-461`](../../crates/desktop/src/git_panel.rs#L422-L461): moving changes between staged/unstaged groups uses those file-level operations.
- [`git.rs:256-309`](../../crates/ide-core/src/git.rs#L256-L309) and [`diff_view.rs:270-321`](../../crates/desktop/src/diff_view.rs#L270-L321): diffs contain display lines and hunk markers but no interactive hunk operations.
- [`git_panel.rs:464-487`](../../crates/desktop/src/git_panel.rs#L464-L487): committing currently stages selected whole files, which must be addressed to preserve partial staging.

## Acceptance criteria

- [ ] Unstaged hunks offer Stage Hunk and staged hunks offer Unstage Hunk, with keyboard-accessible selection/actions.
- [ ] Apply patches against the correct index/worktree snapshot without modifying unrelated staged or unstaged hunks.
- [ ] Detect stale diffs and refresh instead of applying an obsolete hunk; binary/conflicted/unsupported changes have clear unavailable states.
- [ ] Preserve raw patch bytes separately from display-expanded text, including tabs, CRLF and missing-final-newline markers.
- [ ] Refresh both diff sides after an operation, and provide a commit-staged workflow that does not re-stage the remainder of a partially staged file.

## Verification

Stage one of two disjoint hunks, commit it and assert the other remains unstaged. Repeat for unstaging, tabs/CRLF/no-final-newline files and a changed index after preview.
