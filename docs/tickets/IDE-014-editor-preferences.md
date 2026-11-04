# IDE-014 — Add editor preferences, per-language overrides and EditorConfig

**Priority:** P1 · **Area:** Settings/editor · **Size:** M · **Dependencies:** None

## User outcome

Use each project's indentation conventions and adjust editor readability without changing Rust constants.

## Repository evidence

- [`shell.rs:65-70`](../../crates/desktop/src/shell.rs#L65-L70) and [`shell.rs:537-547`](../../crates/desktop/src/shell.rs#L537-L547): indentation is fixed to two spaces, wrapping is disabled and editor text size is constant.
- [`shell.rs:3625-3637`](../../crates/desktop/src/shell.rs#L3625-L3637): the editor consumes the fixed text size/font selection.
- [`settings.rs:24-31`](../../crates/desktop/src/settings.rs#L24-L31): settings have no editor-preferences model.

## Acceptance criteria

- [ ] Add validated settings for font family/size, tab width, tabs versus spaces, word wrap, line numbers and whitespace display.
- [ ] Allow per-language overrides and apply changed preferences to open and newly created editor views.
- [ ] Resolve `.editorconfig` files using normal ancestor/root semantics, supporting at least indentation, line endings, final newline and trailing whitespace policy.
- [ ] Define and document precedence among defaults, user preferences, workspace preferences when available, language overrides and EditorConfig.
- [ ] Indentation/formatting options share the effective settings; preference changes do not silently reformat existing text.

## Verification

Open files governed by nested EditorConfig sections, `root = true`, tabs and language overrides. Reload settings and verify view changes, save-time whitespace behavior and invalid-setting fallback.
