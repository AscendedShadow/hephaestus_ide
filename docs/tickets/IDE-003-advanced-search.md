# IDE-003 — Add regex, whole-word and scope controls to project search

**Priority:** P1 · **Area:** Search · **Size:** M · **Dependencies:** None

## User outcome

Find relevant code accurately across a project and reach every returned match.

## Repository evidence

- [`search.rs:101-141`](../../crates/ide-core/src/search.rs#L101-L141): searches use literal, case-sensitive `match_indices` with no search-options model.
- [`search.rs:8-19`](../../crates/ide-core/src/search.rs#L8-L19) and [`search.rs:29-76`](../../crates/ide-core/src/search.rs#L29-L76): Git ignores are respected, but generated-directory exclusions and the file limit are fixed.
- [`shell.rs:1775-1855`](../../crates/desktop/src/shell.rs#L1775-L1855) and [`shell.rs:2147-2189`](../../crates/desktop/src/shell.rs#L2147-L2189): project matches are capped at 200 and the picker displays/navigates at most 30.
- [`vendor input/search.rs:88-101`](../../vendor/gpui-component/src/input/search.rs#L88-L101): the separate local-find UI already provides case-insensitive literal matching.

## Acceptance criteria

- [ ] Project search offers case sensitivity, whole-word and regex controls, with inline feedback for invalid expressions.
- [ ] Include/exclude globs and a search-within-selected-folder scope are available; ignore behavior is visible and configurable.
- [ ] Results show file groups and match counts, and every result can be reached with mouse or keyboard.
- [ ] Limits/truncation are reported explicitly, with a way to request further results; indexing/search cancellation keeps the UI responsive.
- [ ] Open unsaved content remains the source of truth for open files; Unicode positions remain correct.

## Verification

Cover regex captures, invalid regex, Unicode whole words, include/exclude precedence, ignored files and more than 200 matches. Verify a match beyond the first 30 can be opened.
