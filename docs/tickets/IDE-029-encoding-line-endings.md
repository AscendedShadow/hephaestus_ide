# IDE-029 — Choose file encodings and convert line endings

**Priority:** P2 · **Area:** Documents/editor · **Size:** M · **Dependencies:** None

## User outcome

Open common non-UTF-8 source files and explicitly change encoding or newline conventions when needed.

## Repository evidence

- [`document.rs:57-79`](../../crates/ide-core/src/document.rs#L57-L79): decoding supports UTF-8 with optional BOM and rejects other encodings; LF/CRLF are detected.
- [`document.rs:105-107`](../../crates/ide-core/src/document.rs#L105-L107) and [`document.rs:141-159`](../../crates/ide-core/src/document.rs#L141-L159): line endings are read-only document metadata used during save.
- [`shell.rs:4033-4041`](../../crates/desktop/src/shell.rs#L4033-L4041): the status bar displays UTF-8 and line endings without conversion actions.

## Acceptance criteria

- [ ] Encoding and line-ending status items open explicit Reopen with Encoding, Save with Encoding and LF/CRLF conversion actions.
- [ ] Support UTF-8 with/without BOM and UTF-16 LE/BE; offer an explicit supported legacy encoding selection when automatic decoding is not reliable.
- [ ] Preserve original encoding/BOM/newlines by default and represent explicit conversions as pending document changes even when visible text is unchanged.
- [ ] Reopening a dirty document resolves unsaved content explicitly; failed/unrepresentable conversions retain the original buffer and disk file.
- [ ] Binary detection, external-change protection and save/dirty-state tracking continue to work for supported encodings.

## Verification

Round-trip UTF-16, BOM and CRLF fixtures, convert an unchanged-text document to LF, and attempt an unrepresentable legacy-encoding save. Verify byte output, dirty state and original-file preservation on failure.
