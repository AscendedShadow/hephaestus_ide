# IDE-016 — Add project-specific settings with explicit configuration precedence

**Priority:** P1 · **Area:** Configuration/workspace · **Size:** M · **Dependencies:** None

## User outcome

Keep project run/debug commands, language-server options and editing conventions with the project instead of changing global settings for every repository.

## Repository evidence

- [`settings.rs:189-207`](../../crates/desktop/src/settings.rs#L189-L207): settings resolve to a single per-user OS configuration path.
- [`shell.rs:845-859`](../../crates/desktop/src/shell.rs#L845-L859) and [`shell.rs:931-961`](../../crates/desktop/src/shell.rs#L931-L961): run/debug workflows load that global file.
- [`settings.rs:128-147`](../../crates/desktop/src/settings.rs#L128-L147): language-server overrides exist within one settings object, without layered workspace configuration.

## Acceptance criteria

- [ ] Load a documented workspace configuration location and resolve defaults, user settings and workspace settings with explicit precedence.
- [ ] Define array/map merge semantics and relative-path resolution, especially for commands, language servers and debugger configurations.
- [ ] Provide Open Workspace Settings and Open User Settings commands and show the source of effective values.
- [ ] Switching workspaces applies the new configuration and removes old overrides; existing reload behavior covers both scopes.
- [ ] Malformed workspace settings report their file/location and retain a useful last-known-good configuration.
- [ ] The configuration model distinguishes an omitted value from an explicit reset/disable so defaults do not overwrite unrelated layers.

## Verification

Open two repositories with different commands/server options, reload malformed and corrected configuration, and verify path resolution and removal of prior workspace overrides.
