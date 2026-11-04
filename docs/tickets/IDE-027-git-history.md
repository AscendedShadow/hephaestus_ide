# IDE-027 — Browse repository/file history and line blame

**Priority:** P2 · **Area:** Source control/navigation · **Size:** L · **Dependencies:** None

## User outcome

Understand when and why code changed, and inspect an earlier revision without replacing the working file.

## Repository evidence

- [`git.rs:342-345`](../../crates/ide-core/src/git.rs#L342-L345) and [`git.rs:398-471`](../../crates/ide-core/src/git.rs#L398-L471): repository models expose current status and working/index diffs, without commit/history/blame models.
- [`diff_view.rs:364-443`](../../crates/desktop/src/diff_view.rs#L364-L443): diff presentation is tied to a selected staged/unstaged working-tree file.
- [`commands.rs:68-118`](../../crates/desktop/src/commands.rs#L68-L118): history and blame commands are absent.

## Acceptance criteria

- [ ] Paginate repository history with commit summary, author, date, parents and changed files; filter by a selected file.
- [ ] Open a commit's file changes as read-only revision diffs and open historical content without mutating the worktree or dirty buffers.
- [ ] Follow file renames where supported and distinguish merge-parent comparisons.
- [ ] Show blame for a selected source line/range with a link into commit details; unsaved/uncommitted lines are identified accurately.
- [ ] Provide cancellation, empty-history handling and responsive loading for large repositories.

## Verification

Build a fixture history with a rename, merge and subsequent uncommitted edit. Verify parent-specific diffs, file-history continuity, blame attribution and preservation of an unsaved working buffer.
