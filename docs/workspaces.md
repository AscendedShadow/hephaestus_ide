# Multi-root workspaces

A workspace can hold several folders, for example related repositories.

| Command | What it does |
| --- | --- |
| Add Folder to Workspace | Adds one or more folders (opens the folder when none is open). |
| Remove Folder from Workspace | Removes a folder; its open files stay open, unsaved changes included. |
| Select Active Root | Picks the root used by root-scoped features (also from the status bar). |
| Toggle Root Search | Includes or excludes a root from project search and Quick Open. |
| Save Workspace As | Saves a `.hephaestus-workspace` definition. |

Opening a `.hephaestus-workspace` file (Open File, or a saved session) opens the workspace:

```json
{
  "name": "Team",
  "folders": [
    { "path": "app" },
    { "path": "../shared/lib", "search": false }
  ]
}
```

Folder paths are stored relative to the definition file, so a folder tree moved together
with its definition keeps working. Changes to the roots are written back to the
definition that was opened.

## How roots are used

- **Explorer** shows each root as a top-level folder; roots with the same folder name
  are qualified by their parent folder (`app (one)`, `app (two)`).
- **Quick Open and project search** cover every root that is not excluded from search
  and show root-qualified paths (`lib/src/main.rs`). A folder scope limits the search to
  the root containing it.
- **Language servers** start in the first root and receive every root as a workspace
  folder. Servers that announce workspace-folder change notifications are told when
  roots are added or removed; others are restarted with the new folders.
- **Root-scoped features** — workspace settings (`<root>/.hephaestus/settings.json`),
  editor preferences, Git, the run/debug/task working directory and test discovery —
  use the *active root*. It is chosen explicitly with Select Active Root (default: the
  first root) and does not change when you switch editor tabs. The status bar shows it
  when there are several roots.
- **Unavailable roots** (deleted, unmounted) stay listed with an error instead of
  failing the whole workspace; selecting one in Select Active Root retries it, and
  Remove Folder from Workspace drops it.
- **Sessions** remember the definition file or the list of roots, which roots are
  excluded from search, and the active root.
