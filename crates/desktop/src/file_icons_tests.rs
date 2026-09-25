use std::path::Path;

use super::FileKind;

fn kind(path: &str) -> FileKind {
    FileKind::of(Path::new(path))
}

#[test]
fn detects_extensions() {
    assert_eq!(kind("src/main.rs"), FileKind::Rust);
    assert_eq!(kind("app/index.ts"), FileKind::TypeScript);
    assert_eq!(kind("app/index.tsx"), FileKind::TypeScript);
    assert_eq!(kind("app/index.js"), FileKind::JavaScript);
    assert_eq!(kind("package.json"), FileKind::Json);
    assert_eq!(kind("Cargo.lock"), FileKind::Lock);
    assert_eq!(kind("README.md"), FileKind::Markdown);
    assert_eq!(kind("Cargo.toml"), FileKind::Toml);
}

#[test]
fn detects_gitignore_by_name() {
    assert_eq!(kind(".gitignore"), FileKind::GitIgnore);
    assert_eq!(kind("crates/desktop/.gitignore"), FileKind::GitIgnore);
}

#[test]
fn ignores_extension_case() {
    assert_eq!(kind("NOTES.MD"), FileKind::Markdown);
    assert_eq!(kind("Config.TOML"), FileKind::Toml);
}

#[test]
fn falls_back_to_plain() {
    assert_eq!(kind("notes.txt"), FileKind::Plain);
    assert_eq!(kind("LICENSE"), FileKind::Plain);
    assert_eq!(kind("src"), FileKind::Plain);
}
