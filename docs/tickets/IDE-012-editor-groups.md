# IDE-012 — Add horizontal and vertical split editor groups

**Priority:** P1 · **Area:** Editor layout · **Size:** L · **Dependencies:** None

## User outcome

Read or edit two files, or two locations in the same file, side by side.

## Repository evidence

- [`shell.rs:238-256`](../../crates/desktop/src/shell.rs#L238-L256): one buffer list and active index drive the entire editor area.
- [`shell.rs:216-224`](../../crates/desktop/src/shell.rs#L216-L224): document content and one editor view are coupled in each buffer.
- [`shell.rs:3586-3638`](../../crates/desktop/src/shell.rs#L3586-L3638): rendering selects a single editor or diff view.
- [`shell.rs:4156-4179`](../../crates/desktop/src/shell.rs#L4156-L4179): existing resizable splits divide sidebar/editor/tool panel, not editor groups.

## Acceptance criteria

- [ ] Split Right and Split Down create independently focusable, resizable groups; users can close a group and move a tab between groups.
- [ ] Each group has its own active tab, cursor, selection and scroll state, including multiple views of the same document.
- [ ] Same-document views share content, dirty state and undo history without issuing duplicate LSP document lifecycles.
- [ ] Save, navigation, completion, debugger reveal and close actions target the intended focused group; closing a view does not discard a document still visible elsewhere.
- [ ] Session persistence restores group layout and tabs, and the final view of a dirty document retains the existing save/discard behavior.

## Verification

Edit one document from two groups, undo from either view, move/close tabs and restore the layout. Verify cursor independence, shared content and dirty-document close handling.
