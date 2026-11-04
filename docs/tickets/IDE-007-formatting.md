# IDE-007 — Format documents/selections and support format-on-save

**Priority:** P1 · **Area:** Language services/editor · **Size:** M · **Dependencies:** IDE-005

## User outcome

Apply the project's formatter without leaving the IDE or losing editor state.

## Repository evidence

- [`commands.rs:68-118`](../../crates/desktop/src/commands.rs#L68-L118): there are no Format Document/Selection commands.
- [`language_server.rs:304-365`](../../crates/desktop/src/language_server.rs#L304-L365): formatting and range-formatting requests are absent.
- [`settings.rs:24-31`](../../crates/desktop/src/settings.rs#L24-L31): settings do not include formatter selection or format-on-save.

## Acceptance criteria

- [ ] Format Document and Format Selection use advertised LSP formatting/range-formatting support, with clear feedback if unavailable.
- [ ] Send effective indentation options and apply returned edits through IDE-005 as one undoable operation.
- [ ] Add opt-in format-on-save, with per-language control and a defined formatter selection policy.
- [ ] Formatting preserves cursor/selection as reasonably as possible and handles folded source ranges without modifying projection markers.
- [ ] Save waits for the applicable formatting result; failures and stale responses do not discard edits or report an unsaved document as saved.

## Verification

Return several disjoint edits from a mock formatter, including edits before a Unicode cursor. Cover selection formatting, edits made while a response is pending, failed formatting and format-on-save ordering.
