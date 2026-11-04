# IDE-006 — Connect LSP quick fixes, refactorings and organize imports

**Priority:** P1 · **Area:** Language services · **Size:** M · **Dependencies:** IDE-005

## User outcome

Resolve diagnostics and invoke language-aware refactorings at the cursor or selection.

## Repository evidence

- [`language_server.rs:418-434`](../../crates/desktop/src/language_server.rs#L418-L434): editor attachment installs only a hover provider.
- [`vendor input/lsp/mod.rs:23-51`](../../vendor/gpui-component/src/input/lsp/mod.rs#L23-L51): code-action provider/menu infrastructure is bundled but has no first-party provider attached.
- [`lsp/lib.rs:465-502`](../../crates/lsp/src/lib.rs#L465-L502): advertised text-document capabilities omit code actions.

## Acceptance criteria

- [ ] Quick Fix requests code actions for the source cursor/selection and relevant diagnostics, and presents their titles and disabled reasons.
- [ ] Support literal code actions and command responses, lazy `codeAction/resolve`, and `workspace/executeCommand` as required by returned actions.
- [ ] Apply returned edits through IDE-005, including cross-file edits and subsequent `workspace/applyEdit` requests.
- [ ] Expose Organize Imports and available refactor actions through discoverable commands; use the bundled menu where suitable.
- [ ] Folded display positions map correctly to source ranges, and stale responses cannot affect a different buffer/version.

## Verification

Mock a diagnostic quick fix, a lazily resolved action, a command-only action and a multi-file import change. Verify cancellation, unsupported actions and undo behavior.
