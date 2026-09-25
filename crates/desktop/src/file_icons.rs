use std::{ffi::OsStr, path::Path};

use crate::assets::AppIcon;

#[cfg(test)]
#[path = "file_icons_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    GitIgnore,
    JavaScript,
    Json,
    Lock,
    Markdown,
    Rust,
    Toml,
    TypeScript,
    Plain,
}

impl FileKind {
    pub fn of(path: &Path) -> Self {
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        if name.eq_ignore_ascii_case(".gitignore") {
            return Self::GitIgnore;
        }
        let extension = path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
            .to_ascii_lowercase();
        match extension.as_str() {
            "cjs" | "js" | "jsx" | "mjs" => Self::JavaScript,
            "json" | "jsonc" => Self::Json,
            "lock" => Self::Lock,
            "markdown" | "md" => Self::Markdown,
            "rs" => Self::Rust,
            "toml" => Self::Toml,
            "cts" | "mts" | "ts" | "tsx" => Self::TypeScript,
            _ => Self::Plain,
        }
    }

    pub fn icon(self) -> AppIcon {
        match self {
            Self::GitIgnore => AppIcon::FileIgnored,
            Self::JavaScript => AppIcon::FileJavaScript,
            Self::Json => AppIcon::FileJson,
            Self::Lock => AppIcon::FileLock,
            Self::Markdown => AppIcon::FileMarkdown,
            Self::Rust => AppIcon::FileRust,
            Self::Toml => AppIcon::FileToml,
            Self::TypeScript => AppIcon::FileTypeScript,
            Self::Plain => AppIcon::File,
        }
    }
}
