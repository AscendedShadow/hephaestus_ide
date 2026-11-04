# Editor groups

Split the editor to work on two files, or two places in one file, side by side.

| Command | Default keys |
| --- | --- |
| Split Editor Right | `secondary-\` |
| Split Editor Down | `secondary-k secondary-\` |
| Focus Next / Previous Group | `secondary-k secondary-right` / `secondary-k secondary-left` |
| Move Tab To Next Group | `secondary-k shift-right` (splits right when there is one group) |
| Close Editor Group | `secondary-k w` |

Each group is split off the previous one, to the right or below, and the dividers can be
dragged. Clicking into a group focuses it; commands (save, close, go to definition,
completion, debugger reveal, opening files) act on the focused group, and opening a file
shows it in the focused group.

- A document shown in several groups has one view per group. Views share the text,
  folds, dirty state and undo history (undo from either view), while cursor, selection
  and scroll position are per view. The language server sees the document once.
- Closing a tab that is also shown in another group closes only that view, without a
  prompt. Closing the last view of a modified document asks to save as before.
- Closing a group closes its views of documents shown elsewhere and moves documents
  shown only there into the neighbouring group, so nothing unsaved is lost. An empty
  group closes itself.
- The session remembers the layout, the tabs of each group, each view's cursor and the
  focused group.

The focused group shows the full editor (breakpoint gutter, overlays, diff tab); other
groups show their active document.
