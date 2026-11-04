# IDE-017 — Open and persist multi-root workspaces

**Priority:** P2 · **Area:** Workspace · **Size:** L · **Dependencies:** IDE-002, IDE-016

## User outcome

Work on related repositories or folders in one IDE window with a named, reopenable workspace.

## Repository evidence

- [`workspace.rs:22-43`](../../crates/ide-core/src/workspace.rs#L22-L43): a workspace has one optional root.
- [`session.rs:10-16`](../../crates/desktop/src/session.rs#L10-L16): persistence stores one folder.
- [`language_server.rs:60-78`](../../crates/desktop/src/language_server.rs#L60-L78): language-server state is organized around one root.
- [`shell.rs:787-797`](../../crates/desktop/src/shell.rs#L787-L797): project directory and Git context use one active directory.

## Acceptance criteria

- [ ] Users can add/remove roots and save/open a workspace definition with portable relative paths.
- [ ] Explorer, Quick Open and project search include all roots with unambiguous root-qualified names and optional per-root search scope.
- [ ] Language servers receive correct workspace folders or isolated root-specific instances, according to their support.
- [ ] Run/debug working directories, settings and Git repository selection are explicit per root rather than inferred ambiguously from the last active buffer.
- [ ] Removing a root or reopening a workspace preserves dirty buffers and restores root/group/session state; inaccessible roots have a recoverable error state.

## Verification

Use two Git repositories with duplicate relative filenames and different language-server settings. Remove/re-add one root, move the saved workspace directory and confirm routing/search results remain correct.
