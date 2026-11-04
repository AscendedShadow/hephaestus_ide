# IDE-025 — Search terminal output and scrollback

**Priority:** P2 · **Area:** Terminal · **Size:** M · **Dependencies:** None

## User outcome

Find a previous log message or command result after it has scrolled out of view.

## Repository evidence

- [`terminal/lib.rs:20`](../../crates/terminal/src/lib.rs#L20): the terminal retains 10,000 scrollback lines.
- [`terminal_view.rs:22-24`](../../crates/desktop/src/terminal_view.rs#L22-L24) and [`terminal_view.rs:340-406`](../../crates/desktop/src/terminal_view.rs#L340-L406): terminal actions/rendering expose copy/paste and the grid, without a search workflow.
- [`terminal/snapshot.rs:91-103`](../../crates/terminal/src/snapshot.rs#L91-L103): rendered cells have selection styling but no search-match representation.

## Acceptance criteria

- [ ] A terminal-scoped Find command opens a query bar without sending its keystrokes to the shell process.
- [ ] Search includes retained scrollback, supports case sensitivity, and highlights the current/all visible matches.
- [ ] Next/previous match navigation scrolls to results, reports counts and supports wraparound.
- [ ] Wrapped lines, Unicode/wide/combining characters and incoming output retain correct text-to-grid match mapping.
- [ ] Closing find restores terminal focus without changing the process; evicted history and alternate-screen output have defined behavior.

## Verification

Produce output exceeding the visible grid, including wrapped Unicode lines and repeated matches. Navigate while output arrives and after history eviction; verify shell input is unaffected.
