# IDE-032 — Add standard comment and line-editing commands

**Priority:** P1 · **Area:** Editing/commands · **Size:** M · **Dependencies:** None

## User outcome

Comment code and rearrange lines quickly using discoverable keyboard commands in normal editing mode.

## Repository evidence

- [`vendor input/state.rs:45-90`](../../vendor/gpui-component/src/input/state.rs#L45-L90): bundled actions cover text movement, deletion, indentation and clipboard/history, without toggle-comment or move/duplicate-line actions.
- [`commands.rs:68-118`](../../crates/desktop/src/commands.rs#L68-L118): application commands do not add those operations.
- [`shell.rs:2435-2483`](../../crates/desktop/src/shell.rs#L2435-L2483): editor-specific additions currently include Vim mode and folding; Vim commands are not a substitute for discoverable normal-mode editing commands.

## Acceptance criteria

- [ ] Add Toggle Line Comment and Toggle Block Comment where the current language supports them, using language-aware delimiters.
- [ ] Add Move Line(s) Up/Down, Duplicate Line(s) Up/Down, Delete Line(s) and Insert Line Above/Below.
- [ ] Commands work on the current line or selected whole-line range, preserve meaningful cursor/selection positions and form coherent undo steps.
- [ ] Register commands in the palette/keybinding model with sensible platform defaults and editor-only contexts so terminal shortcuts are unaffected.
- [ ] Handle empty files, end-of-file without newline, Unicode and folded source ranges without modifying hidden projection markers.

## Verification

Exercise mixed-comment selections, multiline moves at both file boundaries, duplication without a final newline and editing around folded blocks. Verify a single undo restores both source and selection.
