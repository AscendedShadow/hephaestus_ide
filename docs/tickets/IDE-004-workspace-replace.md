# IDE-004 — Replace across workspace files with a selectable preview

**Priority:** P1 · **Area:** Search/editing · **Size:** L · **Dependencies:** IDE-003, IDE-005

## User outcome

Apply reviewed textual changes across a project without manually editing each search result.

## Repository evidence

- [`shell.rs:1754-1756`](../../crates/desktop/src/shell.rs#L1754-L1756) and [`shell.rs:1857-1941`](../../crates/desktop/src/shell.rs#L1857-L1941): project search finds and opens matches but has no replacement workflow.
- [`search.rs:101-141`](../../crates/ide-core/src/search.rs#L101-L141): core search returns locations/previews only.
- [`vendor input/search.rs:337-395`](../../vendor/gpui-component/src/input/search.rs#L337-L395): replace-next/all already exist for one editor buffer, so this ticket concerns cross-file replacement.

## Acceptance criteria

- [ ] A replacement field works with IDE-003's search options, including regex capture substitutions.
- [ ] A preview groups changes by file and lets users select individual matches, files or all results.
- [ ] Applying updates open buffers through the shared edit transaction system and persists selected closed-file changes without bypassing external-change checks.
- [ ] Dirty buffers are searched/replaced using their current content; the UI clearly reports which files are modified versus saved.
- [ ] A changed-on-disk or stale preview is detected before applying; failures are reported per file and grouped undo can restore the operation.

## Verification

Replace captures across open dirty and closed files, including Unicode, CRLF and read-only files. Change a file after preview and verify stale changes are rejected rather than overwriting newer content.
