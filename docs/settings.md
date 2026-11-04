# Settings

Hephaestus reads JSON settings from two files and merges them over built-in defaults.

| Layer | Location | Opened with |
| --- | --- | --- |
| Defaults | built in | **Show Effective Settings** shows the merged result |
| User | `$XDG_CONFIG_HOME/hephaestus/settings.json` (Linux), `~/Library/Application Support/Hephaestus/settings.json` (macOS), `%APPDATA%\Hephaestus\settings.json` (Windows) | **Edit settings.json** / **Open User Settings** |
| Workspace | `<workspace root>/.hephaestus/settings.json` | **Open Workspace Settings** (creates the file) |

Saving either file, **Reload Settings**, and switching workspaces re-apply the settings.
Run, task and debug launches re-read both files first, so edits made outside the IDE take
effect.

## Precedence and merging

Later layers win: **defaults < user < workspace < language overrides < EditorConfig**
(the last two only apply to editor preferences, see below).

- **Objects** merge key by key, recursively. A workspace can change one keybinding or
  one theme color without repeating the rest.
- **`null` resets** a key to its built-in default, which is different from omitting the
  key (omitted keys inherit from the lower layer). For example `"debugger": null` in a
  workspace disables a debugger configured in user settings.
- **Named arrays merge by name**: `commands`, `debug_profiles` and `terminal_profiles` by
  `name`, `tasks` by `id`, and `language_servers` by `program`. A workspace entry
  replaces the user entry with the same name; other entries are appended.
- **Other arrays replace** the lower layer's array.

## Relative paths

- In **workspace** settings, `working_directory`/`cwd` values and `program` values that
  start with `./` or `../` are resolved against the workspace root when the file is read.
- In **user** settings, a relative `working_directory` resolves against the active
  project directory (the open folder, or the active file's folder) at launch time.

## Errors

A malformed layer is reported in the status bar with its file, line and column. The last
valid contents of that layer stay in effect until the file is fixed, so a typo never
silently resets your configuration.

## File associations

`file_associations` maps glob patterns to language modes (by id such as `cpp`, or by
name such as `C++`):

```json
{ "file_associations": { "*.h": "cpp", "Jenkinsfile": "java", "config/*.conf": "yaml" } }
```

Patterns without `/` match the file name; patterns with `/` match the
workspace-relative path. `*`, `**`, `?`, `[a-z]` and `{a,b}` are supported. Associations
win over built-in recognition (exact file names, then extensions, then `#!` shebangs).
**Change Language Mode** picks a mode for one buffer, including untitled buffers.

### Language support levels

| Level | Languages |
| --- | --- |
| Highlighting, navigation, default language server | Rust, C, C#, Go, Java, JavaScript/JSX, TypeScript/TSX, Zig |
| Syntax highlighting | C++ (clangd by default), Python, Shell, JSON/JSONC, TOML, YAML, Markdown, HTML, CSS, Makefile |
| Recognized (comments, language server routing) | Dockerfile, ignore files, INI/EditorConfig, plain text |

Any configured `language_servers` entry works for a registered language id.

## Editor preferences

```json
{
  "editor": {
    "font_family": "JetBrains Mono",
    "font_size": 14,
    "tab_width": 2,
    "insert_spaces": true,
    "word_wrap": false,
    "line_numbers": true,
    "show_whitespace": false,
    "trim_trailing_whitespace": false,
    "insert_final_newline": false,
    "line_ending": "lf",
    "format_on_save": false,
    "use_editorconfig": true,
    "languages": {
      "go": { "insert_spaces": false, "tab_width": 8 },
      "python": { "tab_width": 4, "format_on_save": true }
    }
  }
}
```

`font_size` must be 6–72 and `tab_width` 1–16; invalid values are reported and ignored.
`line_ending` applies to files saved for the first time.

Effective editor preferences resolve in this order, later winning:

1. built-in defaults,
2. user `editor`,
3. workspace `editor` (merged with the user layer as described above),
4. `editor.languages.<language id>`,
5. `.editorconfig` (unless `"use_editorconfig": false`).

EditorConfig files are read from the file's folder upwards until one declares
`root = true`; nearer files and later sections win, and `unset` clears an inherited value.
Supported properties: `indent_style`, `indent_size` (including `tab`), `tab_width`,
`end_of_line` (`lf`/`crlf`), `insert_final_newline` and `trim_trailing_whitespace`.

Changing preferences never rewrites existing text. Indentation settings apply to new
indentation; trailing-whitespace and final-newline policies run when a file is saved
(as an undoable edit), and a line ending applies to new files. The status bar shows the
effective indentation. Whitespace markers are drawn for the first visual row of each
line when word wrap is on.

## Saving

**Save All** (File menu, palette, `Ctrl/Cmd+Alt+S`) saves every modified file-backed
buffer with the normal save path. Untitled buffers are never prompted for; the status bar
reports how many remain unsaved.

```json
{ "auto_save": { "mode": "after_delay", "delay_ms": 1000 } }
```

`mode` is `off` (default), `after_delay` (save after `delay_ms` without edits) or
`on_focus_change` (save when the window loses focus or another tab is activated). The
status bar shows the active mode. Auto-save skips untitled buffers and buffers with a
pending external change, never opens prompts or moves focus, serializes saves per file
(edits made during a save stay modified and are saved next), keeps the external-change
and read-only checks of a manual save, and reports a failure once without retrying until
the buffer changes again. Save-time whitespace policies and format-on-save apply to
manual saves only, so text never shifts under the cursor while typing.

## Terminal profiles

```json
{
  "terminal_profiles": [
    { "name": "zsh", "program": "/bin/zsh", "args": ["-l"], "env": { "TERM_THEME": "dark" }, "cwd": "workspace" },
    { "name": "Home bash", "program": "bash", "cwd": "home" },
    { "name": "Logs", "cwd": "var/log" }
  ],
  "default_terminal_profile": "zsh"
}
```

`cwd` is `workspace` (default), `home`, or a path (relative to the workspace folder).
An empty `program` uses the system shell. **New Terminal** (`Ctrl+Shift+``) starts the
default profile; **New Terminal with Profile…** chooses one; **New Terminal Here** in a
folder's context menu starts in that folder. Sessions can be renamed, switched (**Next
Terminal**) and closed; closing a session ends only its process. Switching workspaces
leaves interactive terminals running in their folders. A profile that fails to start names
the profile and the executable.
