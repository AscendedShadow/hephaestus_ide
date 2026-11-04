# IDE-008 — Find references and navigate to implementations and type definitions

**Priority:** P1 · **Area:** Code navigation · **Size:** M · **Dependencies:** None

## User outcome

Understand how a symbol is used and move between interfaces, implementations and types.

## Repository evidence

- [`commands.rs:95-118`](../../crates/desktop/src/commands.rs#L95-L118): go-to-definition and navigation history exist, but references/type/implementation commands do not.
- [`language_server.rs:319-333`](../../crates/desktop/src/language_server.rs#L319-L333) and [`language_server.rs:368-385`](../../crates/desktop/src/language_server.rs#L368-L385): definition navigation reduces a potentially multi-location response to its first entry.

## Acceptance criteria

- [ ] Add Find References, Go to Implementation and Go to Type Definition commands, gated by server capabilities.
- [ ] References are grouped by file with snippets and correct ranges, including an option to include declarations.
- [ ] Multi-target definition/type/implementation responses present a target chooser rather than silently discarding alternatives.
- [ ] Result navigation preserves dirty buffers, reveals folded ranges, and integrates with existing back/forward history.
- [ ] Empty, failed, cancelled and stale requests have distinct behavior and do not move focus unexpectedly.

## Verification

Mock `Location`/`LocationLink` results across files and several overload targets. Verify source mapping, Unicode ranges and history after selecting a non-first target.
