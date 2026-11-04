# IDE-009 — Add a document outline and workspace symbol search

**Priority:** P1 · **Area:** Code navigation · **Size:** M · **Dependencies:** None

## User outcome

Jump directly to functions, types and members without knowing their filenames or line numbers.

## Repository evidence

- [`shell.rs:96-106`](../../crates/desktop/src/shell.rs#L96-L106): picker modes include files, commands and lines, but no symbols.
- [`navigation.rs:313-380`](../../crates/desktop/src/navigation.rs#L313-L380): syntax analysis looks up a name at an offset rather than exposing a browsable outline.
- [`language_server.rs:304-365`](../../crates/desktop/src/language_server.rs#L304-L365): document/workspace symbol requests are not integrated.
- [`shell.rs:3291-3339`](../../crates/desktop/src/shell.rs#L3291-L3339): breadcrumbs display path segments only.

## Acceptance criteria

- [ ] Go to Symbol in File lists hierarchical document symbols, supports filtering, and reveals the selected source range.
- [ ] An outline view shows symbol kind/nesting and tracks the current editor position; editing refreshes it without losing useful selection state.
- [ ] Go to Symbol in Workspace queries the language server and supports location resolution when required.
- [ ] Handle both hierarchical `DocumentSymbol` and flat `SymbolInformation` responses; unsupported languages show an honest unavailable state or an explicitly supported syntax fallback.
- [ ] Symbol jumps integrate with navigation history and folded ranges, and asynchronous results are tied to the originating workspace/document.

## Verification

Use nested types/methods and duplicate symbol names across files. Verify outline updates after unsaved edits and that a pending response from an old document is ignored.
