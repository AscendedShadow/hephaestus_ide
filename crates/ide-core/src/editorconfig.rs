//! `.editorconfig` resolution (https://editorconfig.org).
//!
//! Files are read from the file's directory upwards until a file declares `root = true`.
//! Sections closer to the file override farther ones, and later sections in one file
//! override earlier ones. Supported properties: `indent_style`, `indent_size`,
//! `tab_width`, `end_of_line`, `insert_final_newline`, `trim_trailing_whitespace`
//! (`charset` is read but only reported). Values are case-insensitive; `unset` clears a
//! property inherited from a farther file.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::glob::Glob;

pub const FILE_NAME: &str = ".editorconfig";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IndentStyle {
    Tab,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndOfLine {
    Lf,
    CrLf,
    Cr,
}

/// Properties resolved for one file; `None` means "not specified".
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    pub indent_style: Option<IndentStyle>,
    /// Columns per indentation level (`indent_size = tab` resolves to `tab_width`).
    pub indent_size: Option<usize>,
    pub tab_width: Option<usize>,
    pub end_of_line: Option<EndOfLine>,
    pub insert_final_newline: Option<bool>,
    pub trim_trailing_whitespace: Option<bool>,
    pub charset: Option<String>,
    /// The `.editorconfig` files that contributed, nearest last.
    pub sources: Vec<PathBuf>,
}

impl Properties {
    pub fn is_empty(&self) -> bool {
        self.indent_style.is_none()
            && self.indent_size.is_none()
            && self.tab_width.is_none()
            && self.end_of_line.is_none()
            && self.insert_final_newline.is_none()
            && self.trim_trailing_whitespace.is_none()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Section {
    glob: Option<Glob>,
    properties: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct ConfigFile {
    root: bool,
    sections: Vec<Section>,
}

fn parse(source: &str) -> ConfigFile {
    let mut file = ConfigFile {
        root: false,
        sections: Vec::new(),
    };
    for line in source.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|line| line.strip_suffix(']'))
        {
            file.sections.push(Section {
                glob: section_glob(header),
                properties: Vec::new(),
            });
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim().to_lowercase();
        let value = value.trim().to_string();
        match file.sections.last_mut() {
            Some(section) => section.properties.push((key, value)),
            None if key == "root" => file.root = value.eq_ignore_ascii_case("true"),
            None => {}
        }
    }
    file
}

/// EditorConfig globs: a pattern without `/` matches the file name at any depth, a
/// pattern with `/` is anchored to the directory of the `.editorconfig` file.
fn section_glob(header: &str) -> Option<Glob> {
    let header = header.trim();
    let anchored = header.trim_start_matches('/');
    let pattern = if header.contains('/') {
        anchored.to_string()
    } else {
        header.to_string()
    };
    Glob::new(&pattern).ok()
}

/// Resolve the properties for `file` by reading `.editorconfig` files from disk.
pub fn resolve(file: &Path) -> Properties {
    resolve_with(file, |path| fs::read_to_string(path).ok())
}

/// Resolve with a custom reader (for tests and unsaved trees).
pub fn resolve_with(file: &Path, read: impl Fn(&Path) -> Option<String>) -> Properties {
    let mut files = Vec::new();
    let mut directory = file.parent();
    while let Some(current) = directory {
        let path = current.join(FILE_NAME);
        if let Some(source) = read(&path) {
            let config = parse(&source);
            let root = config.root;
            files.push((current.to_path_buf(), path, config));
            if root {
                break;
            }
        }
        directory = current.parent();
    }
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut sources = Vec::new();
    // Farthest first so nearer files override.
    for (directory, path, config) in files.into_iter().rev() {
        let Ok(relative) = file.strip_prefix(&directory) else {
            continue;
        };
        let mut used = false;
        for section in &config.sections {
            let Some(glob) = &section.glob else { continue };
            let name = relative.file_name().unwrap_or_default().to_string_lossy();
            let matched = if glob.pattern().contains('/') {
                glob.matches(relative)
            } else {
                glob.matches_name(&name)
            };
            if matched {
                for (key, value) in &section.properties {
                    values.insert(key.clone(), value.to_lowercase());
                    used = true;
                }
            }
        }
        if used {
            sources.push(path);
        }
    }
    let get = |key: &str| values.get(key).filter(|value| value.as_str() != "unset");
    let number = |key: &str| {
        get(key)
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|n| *n > 0)
    };
    let boolean = |key: &str| {
        get(key).and_then(|value| match value.as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        })
    };
    let indent_style = get("indent_style").and_then(|value| match value.as_str() {
        "tab" => Some(IndentStyle::Tab),
        "space" => Some(IndentStyle::Space),
        _ => None,
    });
    let tab_width = number("tab_width");
    let indent_size = match get("indent_size").map(String::as_str) {
        Some("tab") => tab_width,
        _ => number("indent_size"),
    };
    Properties {
        indent_style,
        // Per spec, tab_width defaults to indent_size when unspecified.
        tab_width: tab_width.or(indent_size),
        indent_size: indent_size.or(if indent_style == Some(IndentStyle::Tab) {
            tab_width
        } else {
            None
        }),
        end_of_line: get("end_of_line").and_then(|value| match value.as_str() {
            "lf" => Some(EndOfLine::Lf),
            "crlf" => Some(EndOfLine::CrLf),
            "cr" => Some(EndOfLine::Cr),
            _ => None,
        }),
        insert_final_newline: boolean("insert_final_newline"),
        trim_trailing_whitespace: boolean("trim_trailing_whitespace"),
        charset: get("charset").cloned(),
        sources,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn tree(files: &[(&str, &str)]) -> impl Fn(&Path) -> Option<String> {
        let files: HashMap<PathBuf, String> = files
            .iter()
            .map(|(path, text)| (PathBuf::from(path), text.to_string()))
            .collect();
        move |path| files.get(path).cloned()
    }

    #[test]
    fn nearer_files_and_later_sections_win_until_root() {
        let read = tree(&[
            ("/.editorconfig", "[*]\nindent_style = tab\n"),
            (
                "/repo/.editorconfig",
                "root = true\n\n[*]\nindent_style = space\nindent_size = 4\nend_of_line = lf\n\
                 insert_final_newline = true\n\n[*.{js,ts}]\nindent_size = 2\n\n[Makefile]\nindent_style = tab\n",
            ),
            (
                "/repo/web/.editorconfig",
                "[*.ts]\ntrim_trailing_whitespace = true\nend_of_line = unset\n",
            ),
        ]);
        let ts = resolve_with(Path::new("/repo/web/src/app.ts"), &read);
        assert_eq!(ts.indent_style, Some(IndentStyle::Space));
        assert_eq!(ts.indent_size, Some(2));
        assert_eq!(ts.tab_width, Some(2));
        assert_eq!(ts.end_of_line, None, "unset clears an inherited value");
        assert_eq!(ts.trim_trailing_whitespace, Some(true));
        assert_eq!(ts.insert_final_newline, Some(true));
        assert_eq!(ts.sources.len(), 2);

        let make = resolve_with(Path::new("/repo/Makefile"), &read);
        assert_eq!(make.indent_style, Some(IndentStyle::Tab));
        assert_eq!(make.indent_size, Some(4));
        assert_eq!(make.end_of_line, Some(EndOfLine::Lf));

        let outside = resolve_with(Path::new("/other/file.rs"), &read);
        assert_eq!(outside.indent_style, Some(IndentStyle::Tab));
        assert!(resolve_with(Path::new("/x.rs"), tree(&[])).is_empty());
    }

    #[test]
    fn anchored_sections_and_tab_sizes() {
        let read = tree(&[(
            "/p/.editorconfig",
            "root=true\n[src/**.rs]\nindent_size=tab\ntab_width=8\n[/docs/*.md]\ntrim_trailing_whitespace=false\n",
        )]);
        let rust = resolve_with(Path::new("/p/src/a/b.rs"), &read);
        assert_eq!((rust.indent_size, rust.tab_width), (Some(8), Some(8)));
        assert!(
            resolve_with(Path::new("/p/other/b.rs"), &read)
                .indent_size
                .is_none()
        );
        assert_eq!(
            resolve_with(Path::new("/p/docs/a.md"), &read).trim_trailing_whitespace,
            Some(false)
        );
        assert!(resolve_with(Path::new("/p/x/docs/a.md"), &read).is_empty());
    }
}
