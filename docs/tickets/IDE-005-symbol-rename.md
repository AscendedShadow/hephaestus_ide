# IDE-005 — Rename symbols using a shared workspace edit transaction layer

**Priority:** P1 · **Area:** Language services/editing · **Size:** L · **Dependencies:** None

## User outcome

Rename a symbol semantically across its definitions and references, including files that are not open.

## Repository evidence

- [`language_server.rs:304-365`](../../crates/desktop/src/language_server.rs#L304-L365): desktop requests cover hover, definition and completion, without prepare-rename or rename.
- [`lsp/lib.rs:418-455`](../../crates/lsp/src/lib.rs#L418-L455): server-request handling has no `workspace/applyEdit` implementation.
- [`vendor input/lsp/mod.rs:121-135`](../../vendor/gpui-component/src/input/lsp/mod.rs#L121-L135): a buffer-local text-edit helper exists, but there is no first-party multi-document transaction/resource-operation layer.

## Acceptance criteria

- [ ] Rename Symbol is available from the editor and command palette, uses `textDocument/prepareRename` when supported, and prompts with the current symbol name.
- [ ] `textDocument/rename` results support both `changes` and versioned `documentChanges`, including supported create/rename/delete resource operations.
- [ ] A reusable applier validates document versions, ranges, edit ordering/overlap and filesystem destinations before mutation; partial filesystem failures are recoverable and reported accurately.
- [ ] Edits use current dirty-buffer content, preserve Unicode/line-ending correctness, and synchronize editor history, LSP versions and file identity.
- [ ] Multi-file edits have a reviewable preview and a grouped undo/redo operation, with explicit handling of closed files.
- [ ] Server-initiated `workspace/applyEdit` uses the same applier and returns a truthful applied/failure result; advertise only implemented capabilities.

## Verification

Use a mock language server to rename a symbol across two open buffers and one closed file. Include multiple same-file edits, a resource rename, non-BMP text, a stale version and an injected write failure.
