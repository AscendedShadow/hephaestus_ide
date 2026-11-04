# IDE-013 — Support multiple cursors and rectangular selections

**Priority:** P2 · **Area:** Editing · **Size:** L · **Dependencies:** None

## User outcome

Make the same edit at several locations and edit aligned columns efficiently.

## Repository evidence

- [`vendor input/state.rs:267-295`](../../vendor/gpui-component/src/input/state.rs#L267-L295): editor state holds one `selected_range` and one selection direction.
- [`vendor input/selection.rs:45-58`](../../vendor/gpui-component/src/input/selection.rs#L45-L58): word selection updates that single range.
- [`commands.rs:68-118`](../../crates/desktop/src/commands.rs#L68-L118): add-cursor and select-next-occurrence commands are absent.

## Acceptance criteria

- [ ] Users can add/remove carets with a configurable mouse gesture and add carets above/below using commands.
- [ ] Select Next Occurrence and Select All Occurrences create editable selections; overlapping selections merge predictably.
- [ ] Typing, deletion, indentation and paste apply consistently across selections as one undoable operation.
- [ ] Rectangular selection works with tabs, short lines and Unicode using visual columns, with documented soft-wrap behavior.
- [ ] Escape returns to a primary caret; IME, clipboard and existing single-cursor/Vim interactions have explicit, tested behavior.

## Verification

Exercise disjoint and overlapping selections, multiline paste, tabs, non-BMP characters, IME input and undo/redo. Confirm edits are applied against a stable document snapshot.
