# IDE-031 — Add configurable auto-save and a first-class Save All command

**Priority:** P2 · **Area:** Documents/settings · **Size:** M · **Dependencies:** None

## User outcome

Save all work deliberately or keep file-backed buffers saved automatically according to a chosen policy.

## Repository evidence

- [`shell.rs:1442-1525`](../../crates/desktop/src/shell.rs#L1442-L1525): an internal named-buffer `save_all` helper already exists.
- [`commands.rs:68-118`](../../crates/desktop/src/commands.rs#L68-L118) and [`commands.rs:435-449`](../../crates/desktop/src/commands.rs#L435-L449): Save All is not exposed as a registered command/menu action.
- [`settings.rs:24-31`](../../crates/desktop/src/settings.rs#L24-L31): no auto-save policy or delay is configurable.
- [`document.rs:118-123`](../../crates/ide-core/src/document.rs#L118-L123): saves already protect against conflicting disk changes.

## Acceptance criteria

- [ ] Expose Save All through the command registry, palette, File menu and rebindable shortcut, reusing existing save logic.
- [ ] Define handling for untitled buffers and cancelled Save As prompts, reporting which documents remain unsaved.
- [ ] Add disabled-by-default auto-save modes for a configurable idle delay and focus loss; display the effective mode.
- [ ] Automatic saves reuse external-change/read-only checks, LSP save notifications and formatting hooks when available, without stealing focus or launching modal Save As prompts.
- [ ] Debounce and serialize saves per document, preserve newer edits made during an in-flight save, and surface failures without a retry storm.

## Verification

Exercise Save All with named/untitled/read-only buffers and a cancelled path prompt. For auto-save, edit during an in-flight save, introduce an external change and switch focus; verify saved bytes and remaining dirty state.
