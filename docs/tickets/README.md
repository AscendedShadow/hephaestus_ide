# Standard IDE feature backlog

Reviewed **2026-11-03** against the current working tree, including the in-progress editor, diff, and merge changes. These are 32 proposed feature tickets, ready to copy into an issue tracker. The public GitHub issues API returned no existing issues at review time. Tickets are stored locally because the GitHub CLI is unavailable in this environment.

The review covers first-party crates, relevant tests, and the bundled `gpui-component` editor. Evidence links point to the reviewed source; function names provide additional anchors as line numbers change. Findings are based on static inspection, rather than an interactive application walkthrough.

## Existing foundation

| Area | Capabilities found | Remaining work |
| --- | --- | --- |
| Documents | Multiple tabs, dirty tracking, undo/redo, atomic saves, UTF-8 BOM and LF/CRLF preservation, external-change protection | Recovery, auto-save, encoding conversion |
| Explorer | Lazy folder browsing, new untitled buffers, file deletion, OS file-manager reveal, Git decorations | Contextual creation, rename/move, directory operations, live filesystem updates |
| Search/navigation | Quick Open, command palette, literal project search, file search, line/column jumps, back/forward history | Advanced search, workspace replace, symbols, references |
| Editing | Syntax highlighting, indentation, brace guides, brace folding, optional Vim keys | Split groups, multiple cursors, editing preferences, common line commands |
| Find/replace | Bundled editor provides local find, case toggle, replace-next and replace-all | Project replace, regex/whole-word/scope controls |
| Language services | Server configuration/lifecycle, diagnostics, hover, go-to-definition with syntax fallback, manually requested completion | Rename, code actions, formatting, signature help, richer completion |
| Run/debug | Named run commands, one configured debugger, DAP stepping, line breakpoints, stack/scopes/variables, REPL, memory inspection | Task pipelines, test discovery, debug profiles/attach, conditional breakpoints, watches |
| Terminal | PTY-backed shell, task terminal, selection, copy/paste, scrollback, resize, restart after exit | Multiple sessions, profiles, scrollback search |
| Git | Init/clone, status polling, file-level staging/unstaging/revert, selected-file commits, fetch/pull/push, diffs, merge resolution | Branch management, history/blame, hunk-level staging |
| Settings/session | User settings JSON, rebindable application commands, theme customization, saved file/cursor/recent-file session | Workspace settings, multi-root workspaces, unsaved-content recovery |

## Triage conventions

- **P1 — Core workflow:** prioritize for a dependable daily-use IDE.
- **P2 — Productivity/completeness:** implement after the core workflows and their foundations.
- **Size M/L:** relative implementation scope, not a delivery commitment. M is a focused subsystem addition; L crosses subsystems or requires a new state model.
- IDs are local backlog identifiers, not GitHub issue numbers. Dependencies refer to these IDs. Related tickets may be developed together while retaining independently verifiable outcomes.

## Ticket index

| ID | Priority | Size | Feature | Dependencies |
| --- | --- | --- | --- | --- |
| [IDE-001](IDE-001-explorer-file-operations.md) | P1 | M | Contextual file/folder creation, rename and move | — |
| [IDE-002](IDE-002-live-filesystem-sync.md) | P1 | L | Live filesystem synchronization | — |
| [IDE-003](IDE-003-advanced-search.md) | P1 | M | Advanced, scoped project search | — |
| [IDE-004](IDE-004-workspace-replace.md) | P1 | L | Workspace replace with preview | IDE-003, IDE-005 |
| [IDE-005](IDE-005-symbol-rename.md) | P1 | L | Symbol rename and workspace edit transactions | — |
| [IDE-006](IDE-006-code-actions.md) | P1 | M | Quick fixes, refactorings and organize imports | IDE-005 |
| [IDE-007](IDE-007-formatting.md) | P1 | M | Document/selection formatting and format-on-save | IDE-005 |
| [IDE-008](IDE-008-references-navigation.md) | P1 | M | References, implementations and type navigation | — |
| [IDE-009](IDE-009-symbol-navigation.md) | P1 | M | Document outline and symbol search | — |
| [IDE-010](IDE-010-completion.md) | P1 | L | Inline completion with correct edits and snippets | IDE-005 |
| [IDE-011](IDE-011-signature-help.md) | P2 | M | Function signature and parameter help | — |
| [IDE-012](IDE-012-editor-groups.md) | P1 | L | Split editor groups | — |
| [IDE-013](IDE-013-multiple-cursors.md) | P2 | L | Multiple cursors and column selection | — |
| [IDE-014](IDE-014-editor-preferences.md) | P1 | M | Editor preferences and EditorConfig | — |
| [IDE-015](IDE-015-unsaved-recovery.md) | P1 | L | Unsaved-buffer and crash recovery | — |
| [IDE-016](IDE-016-workspace-settings.md) | P1 | M | Workspace-specific settings | — |
| [IDE-017](IDE-017-multi-root-workspaces.md) | P2 | L | Saved multi-root workspaces | IDE-002, IDE-016 |
| [IDE-018](IDE-018-test-explorer.md) | P1 | L | Test discovery, execution and results | — |
| [IDE-019](IDE-019-task-pipelines.md) | P1 | L | Build/task pipelines and pre-debug tasks | IDE-016 |
| [IDE-020](IDE-020-problems-panel.md) | P1 | L | Persistent Problems panel and live build diagnostics | — |
| [IDE-021](IDE-021-debug-profiles-attach.md) | P1 | L | Named debug profiles and process attach | IDE-016 |
| [IDE-022](IDE-022-breakpoint-management.md) | P2 | L | Persistent conditional breakpoints and exception filters | — |
| [IDE-023](IDE-023-watch-expressions.md) | P2 | M | Persistent debugger watch expressions | — |
| [IDE-024](IDE-024-terminal-sessions.md) | P2 | M | Multiple terminal sessions and shell profiles | — |
| [IDE-025](IDE-025-terminal-search.md) | P2 | M | Search terminal scrollback | — |
| [IDE-026](IDE-026-git-branches.md) | P2 | M | Git branch creation and switching | — |
| [IDE-027](IDE-027-git-history.md) | P2 | L | Repository/file history and blame | — |
| [IDE-028](IDE-028-git-hunks.md) | P2 | L | Hunk-level Git staging and unstaging | — |
| [IDE-029](IDE-029-encoding-line-endings.md) | P2 | M | Encoding and line-ending conversion | — |
| [IDE-030](IDE-030-language-registry.md) | P1 | L | Project-file language support and associations | — |
| [IDE-031](IDE-031-autosave-save-all.md) | P2 | M | Configurable auto-save and explicit Save All | — |
| [IDE-032](IDE-032-editing-commands.md) | P1 | M | Comment, move, duplicate and delete line commands | — |

## Suggested delivery order

1. **Foundations:** IDE-005 (shared edit transactions), IDE-016 (workspace settings), IDE-002 (filesystem updates), IDE-015 (recovery), IDE-014 (editor preferences).
2. **Everyday coding:** IDE-001, IDE-003/004, IDE-006/007/008/009/010, IDE-012, IDE-030, IDE-032.
3. **Build/test/debug loop:** IDE-018, IDE-019, IDE-020, IDE-021.
4. **Productivity/completeness:** remaining P2 tickets, respecting the dependencies above.

This is a baseline desktop IDE backlog; each ticket describes an observable user workflow and the specific missing integration behind it.
