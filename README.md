# Hephaestus

A Rust-native IDE built with GPUI, aiming for a focused Zed-inspired editing
experience and JetBrains-inspired project navigation and tool panels.

## Current state

The desktop application supports one editable document at a time:

- New documents and native file-open / Save As dialogs.
- Multiline text editing with line numbers, selection, copy/cut/paste, undo/redo,
  search, and horizontal/vertical scrolling through GPUI Component's editor.
- Save, modified-document indicators, and Save / Discard / Cancel confirmation
  before replacing a document or closing the window.
- Background file I/O with atomic replacement on save.
- Dark editor, toolbar, panels, and a custom dark title bar with window controls,
  independent of the operating system's light/dark setting. Native file pickers
  follow the operating system's theme.

This first editor supports UTF-8 files up to 8 MiB. UTF-8 BOMs and uniform CRLF
line endings are preserved; mixed CRLF/LF files normalize to LF when saved.
Binary and non-UTF-8 files show an error without replacing the current document.
Editing pauses during file operations and unsaved-change confirmation.

Project directory loading, multiple document tabs, syntax highlighting, PTYs,
Git, and debugging are not implemented yet. Terminal / Git / Debug remain
labeled placeholders. External file-change detection is a future workspace feature.

### Keyboard shortcuts

Use **Cmd** in place of **Ctrl** on macOS unless noted.

| Action | Shortcut |
| --- | --- |
| New document | Ctrl+N |
| Open file | Ctrl+O |
| Save | Ctrl+S |
| Save As | Ctrl+Shift+S |
| Close window | Ctrl+Shift+W |
| Select all / copy / cut / paste | Ctrl+A / C / X / V |
| Undo | Ctrl+Z |
| Redo | Ctrl+Y (macOS: Cmd+Shift+Z) |
| Find | Ctrl+F |

## Development

Install Rust through rustup. `rust-toolchain.toml` pins the toolchain and includes
rustfmt and Clippy. GPUI is pinned to the published **0.2.2** release, which uses
`Application::new()` and includes platform integration in the `gpui` crate.
Examples on GPUI's main branch may use newer, incompatible APIs.
The editor uses **gpui-component 0.5.0**, compatible with this GPUI release;
the optional WebView and language-grammar packs are disabled.

```sh
cargo run -p hephaestus
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release -p hephaestus
```

The release executable is in `target/release/`. Commit `Cargo.lock` for
reproducible application dependency resolution.

### Platform prerequisites

- **Windows:** Visual Studio Build Tools with the Desktop development with C++
  workload and a Windows SDK; use the MSVC Rust toolchain.
- **macOS:** Xcode and its command-line tools, including the Metal build tools.
- **Linux:** development libraries for the selected Wayland/X11 backends and
  text/graphics stack. Follow the build prerequisites matching GPUI 0.2.2 and
  your distribution; both windowing backends are currently enabled by default.
- A graphics environment supported by GPUI is required to run the application.

## Structure

```text
crates/
  ide-core/                 Framework-independent domain state
    src/document.rs         Rope snapshots, encoding, file loading and persistence
    src/workspace.rs        Workspace identity
  desktop/                  GPUI executable (hephaestus)
    src/main.rs             Application and window lifecycle
    src/shell.rs            Document workflow, layout, and tool-panel selection
    src/commands.rs         Application actions and keyboard shortcuts
    src/shell_tests.rs      Headless GPUI editing and save-flow tests
    src/theme.rs            Shared dark-theme colors
docs/
  architecture.md           Subsystem boundaries and implementation order
```

Start small: extract additional crates when their implementations establish
clear boundaries rather than adding empty service crates now.
