# IDE-011 — Show function signatures and active parameter help

**Priority:** P2 · **Area:** Language services · **Size:** M · **Dependencies:** None

## User outcome

See a function's parameters and overloads while writing a call.

## Repository evidence

- [`language_server.rs:304-365`](../../crates/desktop/src/language_server.rs#L304-L365): hover and completion are available, without a signature-help request or model.
- [`lsp/lib.rs:473-499`](../../crates/lsp/src/lib.rs#L473-L499): text-document capabilities do not advertise signature help.
- [`commands.rs:95-118`](../../crates/desktop/src/commands.rs#L95-L118): there is no explicit parameter-help command.

## Acceptance criteria

- [ ] Request signature help on advertised trigger/retrigger characters and through a manual command.
- [ ] A caret-anchored popover displays the active signature, highlights the active parameter, and provides parameter/signature documentation.
- [ ] Users can navigate overloads; parameter labels support both string labels and protocol offset pairs.
- [ ] Moving out of a call or pressing Escape dismisses help; typing updates it without stealing focus or fighting completion.
- [ ] Requests use source positions through folds and discard stale results after document/cursor changes.

## Verification

Mock overloaded calls, nested calls, Unicode parameter labels and comma-triggered updates. Verify focus remains in the editor and old responses cannot restore a dismissed popover.
