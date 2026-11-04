# IDE-002 — Keep the explorer, search index and open files synchronized with disk

**Priority:** P1 · **Area:** Workspace · **Size:** L · **Dependencies:** None

## User outcome

See changes made by generators, Git commands and other applications while the IDE remains focused.

## Repository evidence

- [`shell.rs:398-403`](../../crates/desktop/src/shell.rs#L398-L403): open-document disk checks run on window activation.
- [`shell.rs:698-760`](../../crates/desktop/src/shell.rs#L698-L760): clean documents reload and dirty documents receive an external-change flag; this existing conflict protection should be reused.
- [`shell.rs:1709-1733`](../../crates/desktop/src/shell.rs#L1709-L1733) and [`workspace.rs:22-26`](../../crates/ide-core/src/workspace.rs#L22-L26): search paths and expanded listings are cached snapshots.
- [`git_panel.rs:129-143`](../../crates/desktop/src/git_panel.rs#L129-L143): Git polling exists independently of explorer/document refresh.

## Acceptance criteria

- [ ] External file/folder creation, modification, rename and deletion update visible listings and Quick Open/project-search candidates without reopening the workspace.
- [ ] Clean open buffers reload with cursor/scroll preservation; dirty buffers retain their contents and expose the existing conflict workflow.
- [ ] Deleted or renamed open files have an explicit, consistent buffer state and cannot silently recreate an obsolete path.
- [ ] Events are debounced, own saves are coalesced, and workspace switches cancel old subscriptions and discard stale events.
- [ ] Watch failures/overflow trigger a rescan or a discoverable manual Refresh command; large generated trees do not block the UI.

## Verification

Modify a temporary workspace from a second process while the window stays active. Cover atomic-save rename patterns, event bursts, dirty files, deletion and switching roots during pending events.
