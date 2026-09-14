# Architecture

## Dependency direction

`desktop -> ide-core`. Core code must not depend on GPUI. GPUI entities own
presentation state (focus, active tabs, tool panels); the core owns domain state.
Use GPUI's text and paint APIs for the editor and terminal surfaces rather than
creating a widget per character or a second graphics renderer.

## Planned subsystems

| Subsystem | Responsibility | Initial integration |
| --- | --- | --- |
| Editor | Documents, edit transactions, undo, selections | Rope buffer; custom GPUI element |
| Workspace | Root discovery, file tree, watching, search | `ignore`, `notify`, Cargo metadata |
| Language | Server lifecycle, versioned diagnostics and edits | LSP and rust-analyzer |
| Terminal | PTY lifecycle and terminal grid | `portable-pty`, `alacritty_terminal` |
| Git | Status, diffs, staging, commits | Git CLI with machine-readable output |
| Debug | Session lifecycle, breakpoints, stack and variables | DAP and CodeLLDB |
| Tasks | Cargo builds, tests, executable discovery | Cargo JSON output |

Workspace identity, document persistence, and a single-document editing workflow
exist today. Add the remaining subsystems as core modules or dedicated crates
when implemented; keep protocol/process code independent from the desktop crate.

### Initial editor integration

`gpui-component` 0.5.0 supplies the input surface and its selection, IME, undo,
clipboard, and scrolling behavior. `InputState` lives in the desktop crate.
On text changes, `ide-core::Document` receives a cheap clone of its Rope snapshot;
it keeps the saved snapshot to detect unsaved changes, including undo back to
the saved content. This is a deliberate initial adapter boundary: a future
custom editor can replace `InputState` without changing persistence.

Document reads and atomic saves run on GPUI's background executor. The editor
pauses input during file dialogs and I/O, and switching/closing a dirty document
requires Save / Discard / Cancel. Save failures keep the document and pending
confirmation intact. Each loaded/new document gets a new input entity so undo
history cannot cross document boundaries. Saving never replaces the live input
entity, so its selection and undo history survive a save.

The UI supports UTF-8 with optional BOM and preserves uniform LF/CRLF. It uses
an 8 MiB open limit for this basic editor. Mixed CRLF/LF becomes LF on save.
External modifications are not watched yet; conflict handling belongs with the
future workspace watcher.

## Execution model

- Input, layout, and painting stay on the GPUI UI thread.
- Filesystem access, parsing, Git, builds, and protocol I/O run in background
  workers. Never wait for a process or filesystem scan inside `Render`.
- Use GPUI foreground/background executors for UI integration. If protocol
  services need Tokio, host that runtime separately and bridge with bounded
  channels; a GPUI task is not automatically a Tokio runtime context.
- Version document snapshots and discard stale background results.
- Cancel work when its document, query, or workspace becomes obsolete.
- Coalesce terminal and filesystem updates before requesting UI redraws.
- Delegate glyph rasterization/caching to GPUI; cache editor line layouts above
  it and render only visible content plus a small overscan region.

## Implementation order

1. Extend current commands/file picker with a project picker and asynchronous directory scan.
2. Extend single-document open/save and editing with tabs and editor customization.
3. Incremental syntax highlighting and rust-analyzer integration.
4. PTY terminal and Cargo build/run/test workflows.
5. Git status, diff view, staging, and commits.
6. DAP sessions, breakpoints, execution controls, and variable inspection.
7. Persistence, accessibility, IME validation, performance budgets, packaging.

Validate IME and focus behavior while building the editor, not only at release.
Keep the shell's placeholder messages until the corresponding functionality is
actually available.
