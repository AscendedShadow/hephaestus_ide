# IDE-030 — Recognize common project files and allow language associations

**Priority:** P1 · **Area:** Languages/configuration · **Size:** L · **Dependencies:** None

## User outcome

Edit project configuration, documentation and common source files with the correct language mode and available language services.

## Repository evidence

- [`syntax.rs:101-130`](../../crates/desktop/src/syntax.rs#L101-L130): grammar selection and extension detection are hardcoded to C, C#, Go, Java, JS/TS, Rust and Zig; other files become `plain_text`.
- [`language_server.rs:387-400`](../../crates/desktop/src/language_server.rs#L387-L400): LSP language IDs use another fixed mapping, falling back to `plaintext`.
- [`settings.rs:58-66`](../../crates/desktop/src/settings.rs#L58-L66): arbitrary language-server names can be configured, but file recognition cannot map new languages to them.

## Acceptance criteria

- [ ] Introduce a shared language registry for file extensions, exact filenames, shebang rules, language IDs, comment syntax and available grammar/server support.
- [ ] Recognize common project/configuration files: JSON/JSONC, TOML, YAML, Markdown, HTML/CSS, shell scripts, Python and C++, with documented levels of support.
- [ ] Provide syntax highlighting for the listed modes using suitable existing or added grammars; extensionless files such as Dockerfile/Makefile have explicit recognition rules.
- [ ] Let users override file associations and manually select a buffer's language mode, including untitled buffers.
- [ ] Changes in mode update highlighting, applicable editing behavior and the correct LSP document lifecycle; configurable servers work for registered language IDs.

## Verification

Open representative filenames/extensions, a shebang script and an untitled buffer with manual mode selection. Override an association and verify both the grammar and LSP `languageId`, including C versus C++ headers.
