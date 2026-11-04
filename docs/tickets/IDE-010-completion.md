# IDE-010 — Provide inline completion with exact edits, auto-imports and snippets

**Priority:** P1 · **Area:** Language services/editor · **Size:** L · **Dependencies:** IDE-005

## User outcome

Accept context-aware suggestions at the caret while continuing to type, including imports and function argument placeholders.

## Repository evidence

- [`shell.rs:2619-2666`](../../crates/desktop/src/shell.rs#L2619-L2666): explicit completion converts items to label/text pairs, drops text containing `$`, and opens the general search picker.
- [`shell.rs:2014-2034`](../../crates/desktop/src/shell.rs#L2014-L2034): insertion replaces a locally inferred word prefix rather than the server's edit range.
- [`language_server.rs:335-355`](../../crates/desktop/src/language_server.rs#L335-L355): requests omit completion context and discard list incompleteness metadata.
- [`lsp/lib.rs:486-496`](../../crates/lsp/src/lib.rs#L486-L496): snippet support is explicitly false; the bundled editor has an unused completion-provider integration point.

## Acceptance criteria

- [ ] Show a caret-anchored completion menu on configured typing/server trigger characters and manual invocation, without moving typing into the general search input.
- [ ] Retain full completion items/list metadata, apply `textEdit` or insert/replace ranges exactly, and respect filtering, sorting and incomplete-list refresh semantics.
- [ ] Resolve item details/documentation when needed and apply `additionalTextEdits` and supported completion commands, including auto-imports.
- [ ] Support snippet placeholders/tab stops and advertise snippet support only once insertion/navigation are implemented; literal `$` text remains valid.
- [ ] Acceptance is undoable as a coherent operation; cancellation and document/version checks prevent stale insertions.

## Verification

Mock a member completion replacing a non-prefix range, an auto-import, an insert/replace item, an incomplete list, a snippet and a literal dollar-containing identifier. Verify typing/filtering and one-step undo.
