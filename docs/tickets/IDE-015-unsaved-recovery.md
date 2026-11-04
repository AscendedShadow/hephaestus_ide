# IDE-015 — Recover unsaved and untitled buffers after restart or crash

**Priority:** P1 · **Area:** Persistence · **Size:** L · **Dependencies:** None

## User outcome

Recover work that was never saved to its project file, including untitled documents.

## Repository evidence

- [`session.rs:8-16`](../../crates/desktop/src/session.rs#L8-L16): session state contains paths/cursors/recent files, without buffer content or recovery metadata.
- [`shell.rs:443-501`](../../crates/desktop/src/shell.rs#L443-L501): restore reads disk files and takes only 12; saving excludes untitled buffers.
- [`shell.rs:1178-1188`](../../crates/desktop/src/shell.rs#L1178-L1188) and [`shell.rs:1217-1227`](../../crates/desktop/src/shell.rs#L1217-L1227): session persistence is tied to normal window close paths.

## Acceptance criteria

- [ ] Debounced recovery snapshots preserve dirty file-backed and untitled buffer content outside project files while the application is running.
- [ ] Restart restores recoverable buffers, identities, ordering and cursor state, with a clear recovered-content indicator.
- [ ] If a project file changed since the snapshot, preserve both versions and provide a comparison/reconciliation path.
- [ ] Recovery storage is atomic and versioned; interrupted writes or one corrupt entry do not prevent recovery of other buffers.
- [ ] Successful saves and explicit discards clean up the appropriate snapshots; restoring sessions does not silently drop tabs beyond the current 12-file limit.

## Verification

Force-terminate the application after editing saved and untitled files, then restart. Include a concurrently modified disk file, a truncated snapshot and more than 12 tabs; verify explicit discard prevents resurrection.
