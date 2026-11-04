# IDE-001 — Create, rename and move files/folders from the explorer

**Priority:** P1 · **Area:** Explorer · **Size:** M · **Dependencies:** None

## User outcome

Manage project structure directly in the IDE, with new items created in the selected directory.

## Repository evidence

- [`shell.rs:3066-3102`](../../crates/desktop/src/shell.rs#L3066-L3102): the explorer's New File button creates an untitled editor buffer.
- [`shell.rs:3170-3271`](../../crates/desktop/src/shell.rs#L3170-L3271): folder context menus are empty; file menus expose View, Reveal and Delete. File deletion already exists.
- [`workspace.rs:22-100`](../../crates/ide-core/src/workspace.rs#L22-L100): the workspace model manages listings/expansion without create, rename or move operations.

## Acceptance criteria

- [ ] Explorer commands create a named file or folder under the selected directory, including an empty workspace root.
- [ ] Files and directories can be renamed and moved through explorer commands; collisions and invalid names produce actionable errors.
- [ ] Moving/renaming an open file or directory updates descendant buffer paths, tabs, language-server document identities, search entries and breakpoint paths without losing edits.
- [ ] Directory deletion is supported with an explicit disposition for open dirty descendants and a clear error if the operation fails.
- [ ] Operations are keyboard-accessible, preserve unaffected tree expansion, and refresh Git decorations and Quick Open.

## Verification

Exercise create/rename/move/delete in a temporary workspace containing nested directories, Unicode names, an existing destination and a dirty open descendant. Verify on-disk results and editor path/state consistency.
