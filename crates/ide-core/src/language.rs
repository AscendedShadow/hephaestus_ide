//! Shared language registry: file recognition, LSP identifiers, comment syntax and the
//! level of support the IDE offers for each language mode.
//!
//! Detection precedence (first match wins):
//! 1. a user association whose glob matches the path (see [`Associations`]),
//! 2. an exact file name (`Makefile`, `Dockerfile`, `Cargo.lock`, …),
//! 3. the file extension (longest compound extension first, e.g. `d.ts` before `ts`),
//! 4. a `#!` shebang on the first line,
//! 5. plain text.

use std::path::Path;

use crate::glob::Glob;

pub const PLAIN_TEXT: &str = "plain_text";

/// How much the IDE supports a language mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Support {
    /// Recognized for comments and language-server routing, no syntax grammar.
    Recognized,
    /// Syntax highlighting is available.
    Highlighting,
    /// Highlighting plus syntax-based navigation and a default language server.
    Full,
}

impl Support {
    pub fn label(self) -> &'static str {
        match self {
            Self::Recognized => "recognized",
            Self::Highlighting => "syntax highlighting",
            Self::Full => "highlighting, navigation and language server",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    /// Internal mode identifier used for grammars and settings (`"rust"`, `"c_sharp"`).
    pub id: &'static str,
    /// Human-readable name.
    pub name: &'static str,
    /// LSP `languageId` sent in `textDocument/didOpen`.
    pub lsp_id: &'static str,
    pub extensions: &'static [&'static str],
    pub filenames: &'static [&'static str],
    /// Interpreter names recognized after `#!` (also through `/usr/bin/env`).
    pub interpreters: &'static [&'static str],
    pub line_comment: Option<&'static str>,
    pub block_comment: Option<(&'static str, &'static str)>,
    pub support: Support,
}

const fn language(
    id: &'static str,
    name: &'static str,
    lsp_id: &'static str,
    support: Support,
) -> Language {
    Language {
        id,
        name,
        lsp_id,
        extensions: &[],
        filenames: &[],
        interpreters: &[],
        line_comment: None,
        block_comment: None,
        support,
    }
}

const C_COMMENTS: Comments = (Some("//"), Some(("/*", "*/")));

macro_rules! lang {
    ($id:literal, $name:literal, $lsp:literal, $support:ident,
     ext: [$($ext:literal),*], files: [$($file:literal),*], shebang: [$($sh:literal),*],
     comments: $comments:expr) => {
        Language {
            extensions: &[$($ext),*],
            filenames: &[$($file),*],
            interpreters: &[$($sh),*],
            line_comment: $comments.0,
            block_comment: $comments.1,
            ..language($id, $name, $lsp, Support::$support)
        }
    };
}

type Comments = (Option<&'static str>, Option<(&'static str, &'static str)>);
const HASH: Comments = (Some("#"), None);
const SLASHES: Comments = (Some("//"), None);
const MARKUP: Comments = (None, Some(("<!--", "-->")));
const BLOCK: Comments = (None, Some(("/*", "*/")));
const NONE: Comments = (None, None);

pub static LANGUAGES: &[Language] = &[
    lang!("rust", "Rust", "rust", Full, ext: ["rs"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("c", "C", "c", Full, ext: ["c", "h"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("cpp", "C++", "cpp", Highlighting,
        ext: ["cpp", "cc", "cxx", "c++", "hpp", "hh", "hxx", "h++", "ipp", "inl"],
        files: [], shebang: [], comments: C_COMMENTS),
    lang!("c_sharp", "C#", "csharp", Full, ext: ["cs", "csx"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("go", "Go", "go", Full, ext: ["go"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("java", "Java", "java", Full, ext: ["java"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("javascript", "JavaScript", "javascript", Full, ext: ["js", "mjs", "cjs"],
        files: [], shebang: ["node", "nodejs", "deno", "bun"], comments: C_COMMENTS),
    lang!("jsx", "JavaScript JSX", "javascriptreact", Full, ext: ["jsx"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("typescript", "TypeScript", "typescript", Full, ext: ["ts", "mts", "cts", "d.ts"],
        files: [], shebang: ["ts-node", "tsx"], comments: C_COMMENTS),
    lang!("tsx", "TypeScript JSX", "typescriptreact", Full, ext: ["tsx"], files: [], shebang: [], comments: C_COMMENTS),
    lang!("zig", "Zig", "zig", Full, ext: ["zig", "zon"], files: [], shebang: [], comments: SLASHES),
    lang!("python", "Python", "python", Highlighting, ext: ["py", "pyi", "pyw"],
        files: ["SConstruct", "SConscript"], shebang: ["python", "python2", "python3", "pypy", "pypy3"],
        comments: HASH),
    lang!("shell", "Shell Script", "shellscript", Highlighting,
        ext: ["sh", "bash", "zsh", "ksh", "ebuild", "eclass"],
        files: [".bashrc", ".bash_profile", ".bash_aliases", ".profile", ".zshrc", ".zprofile", ".envrc", "PKGBUILD"],
        shebang: ["sh", "bash", "zsh", "ksh", "dash", "ash"], comments: HASH),
    lang!("json", "JSON", "json", Highlighting,
        ext: ["json", "geojson", "webmanifest", "jsonl"],
        files: [".prettierrc", ".babelrc", ".eslintrc", "composer.lock", "flake.lock"], shebang: [], comments: NONE),
    lang!("jsonc", "JSON with Comments", "jsonc", Highlighting, ext: ["jsonc", "code-workspace"],
        files: ["tsconfig.json", "jsconfig.json", "settings.json", "launch.json", "tasks.json",
                "devcontainer.json", ".devcontainer.json", "keybindings.json", ".hephaestus.json"],
        shebang: [], comments: C_COMMENTS),
    lang!("toml", "TOML", "toml", Highlighting, ext: ["toml"],
        files: ["Cargo.lock", "Pipfile", "poetry.lock", "uv.lock", ".editorconfig.toml"], shebang: [], comments: HASH),
    lang!("yaml", "YAML", "yaml", Highlighting, ext: ["yaml", "yml"],
        files: [".clang-format", ".clang-tidy", ".gitlab-ci.yml"], shebang: [], comments: HASH),
    lang!("markdown", "Markdown", "markdown", Highlighting,
        ext: ["md", "markdown", "mdown", "mkd", "mdx"], files: [], shebang: [],
        comments: MARKUP),
    lang!("html", "HTML", "html", Highlighting, ext: ["html", "htm", "xhtml", "shtml"],
        files: [], shebang: [], comments: MARKUP),
    lang!("css", "CSS", "css", Highlighting, ext: ["css"], files: [], shebang: [],
        comments: BLOCK),
    lang!("makefile", "Makefile", "makefile", Highlighting, ext: ["mk", "mak", "make"],
        files: ["Makefile", "makefile", "GNUmakefile", "BSDmakefile"], shebang: ["make"], comments: HASH),
    lang!("dockerfile", "Dockerfile", "dockerfile", Recognized, ext: ["dockerfile"],
        files: ["Dockerfile", "Containerfile"], shebang: [], comments: HASH),
    lang!("ignore", "Ignore File", "ignore", Recognized, ext: [],
        files: [".gitignore", ".dockerignore", ".npmignore", ".prettierignore", ".eslintignore", ".ignore"],
        shebang: [], comments: HASH),
    lang!("ini", "INI / EditorConfig", "ini", Recognized, ext: ["ini", "cfg", "conf", "properties"],
        files: [".editorconfig", ".gitconfig", ".gitattributes", ".npmrc"], shebang: [], comments: HASH),
    lang!("plain_text", "Plain Text", "plaintext", Recognized, ext: ["txt", "text", "log"],
        files: ["LICENSE", "COPYING"], shebang: [], comments: NONE),
];

/// User file-association overrides: `glob -> language id`, checked in order.
#[derive(Clone, Debug, Default)]
pub struct Associations {
    rules: Vec<(Glob, &'static str)>,
}

impl Associations {
    /// Build associations, returning a description of each invalid rule.
    pub fn new<'a>(rules: impl IntoIterator<Item = (&'a str, &'a str)>) -> (Self, Vec<String>) {
        let mut problems = Vec::new();
        let mut parsed = Vec::new();
        for (pattern, id) in rules {
            let Some(language) = by_id(id).or_else(|| by_name(id)) else {
                problems.push(format!(
                    "file_associations: unknown language {id:?} for {pattern:?}"
                ));
                continue;
            };
            match Glob::new(pattern) {
                Ok(glob) => parsed.push((glob, language.id)),
                Err(error) => problems.push(format!("file_associations: {error}")),
            }
        }
        (Self { rules: parsed }, problems)
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    fn lookup(&self, path: &Path) -> Option<&'static str> {
        let name = path.file_name()?.to_string_lossy();
        self.rules
            .iter()
            .find(|(glob, _)| glob.matches_name(&name) || glob.matches(path))
            .map(|(_, id)| *id)
    }
}

pub fn all() -> &'static [Language] {
    LANGUAGES
}

pub fn by_id(id: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|language| language.id == id)
}

/// Look up by display name or LSP id, case-insensitively.
pub fn by_name(name: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|language| {
        language.name.eq_ignore_ascii_case(name) || language.lsp_id.eq_ignore_ascii_case(name)
    })
}

pub fn get(id: &str) -> &'static Language {
    by_id(id).unwrap_or_else(|| by_id(PLAIN_TEXT).expect("plain text is registered"))
}

/// Detect the language of a file from its path and (optionally) its first line.
pub fn detect(
    path: Option<&Path>,
    first_line: Option<&str>,
    associations: &Associations,
) -> &'static str {
    if let Some(path) = path {
        if let Some(id) = associations.lookup(path) {
            return id;
        }
        if let Some(id) = from_path(path) {
            return id;
        }
    }
    first_line.and_then(from_shebang).unwrap_or(PLAIN_TEXT)
}

/// Recognize a language by exact file name or extension.
pub fn from_path(path: &Path) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy();
    if let Some(language) = LANGUAGES
        .iter()
        .find(|language| language.filenames.iter().any(|file| *file == name))
    {
        return Some(language.id);
    }
    // Dockerfile.dev, Makefile.am …
    if let Some((stem, _)) = name.split_once('.')
        && let Some(language) = LANGUAGES.iter().find(|language| {
            language.filenames.iter().any(|file| *file == stem) && !stem.starts_with('.')
        })
        && matches!(language.id, "dockerfile" | "makefile")
    {
        return Some(language.id);
    }
    let lower = name.to_lowercase();
    // Longest compound extension first (`d.ts`, then `ts`).
    let mut dots: Vec<usize> = lower.match_indices('.').map(|(ix, _)| ix).collect();
    dots.retain(|&ix| ix > 0 || lower.len() > 1);
    for dot in dots {
        let extension = &lower[dot + 1..];
        if let Some(language) = LANGUAGES
            .iter()
            .find(|language| language.extensions.contains(&extension))
        {
            return Some(language.id);
        }
    }
    None
}

/// Recognize a `#!` interpreter line.
pub fn from_shebang(line: &str) -> Option<&'static str> {
    let command = line.strip_prefix("#!")?.trim();
    let mut words = command.split_whitespace();
    let mut program = words.next()?.rsplit('/').next()?;
    if program == "env" {
        program = words.find(|word| !word.starts_with('-') && !word.contains('='))?;
    }
    let program = program.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.');
    LANGUAGES
        .iter()
        .find(|language| {
            language.interpreters.iter().any(|interpreter| {
                interpreter.trim_end_matches(|c: char| c.is_ascii_digit()) == program
            })
        })
        .map(|language| language.id)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn detect_path(path: &str) -> &'static str {
        detect(Some(Path::new(path)), None, &Associations::default())
    }

    #[test]
    fn recognizes_project_files_by_name_and_extension() {
        assert_eq!(detect_path("src/main.rs"), "rust");
        assert_eq!(detect_path("Cargo.toml"), "toml");
        assert_eq!(detect_path("Cargo.lock"), "toml");
        assert_eq!(detect_path("package.json"), "json");
        assert_eq!(detect_path("tsconfig.json"), "jsonc");
        assert_eq!(detect_path(".github/workflows/ci.yml"), "yaml");
        assert_eq!(detect_path("README.md"), "markdown");
        assert_eq!(detect_path("index.HTML"), "html");
        assert_eq!(detect_path("site.css"), "css");
        assert_eq!(detect_path("build.sh"), "shell");
        assert_eq!(detect_path("tool.py"), "python");
        assert_eq!(detect_path("widget.hpp"), "cpp");
        assert_eq!(detect_path("types.d.ts"), "typescript");
        assert_eq!(detect_path("Dockerfile"), "dockerfile");
        assert_eq!(detect_path("Dockerfile.dev"), "dockerfile");
        assert_eq!(detect_path("Makefile"), "makefile");
        assert_eq!(detect_path("GNUmakefile"), "makefile");
        assert_eq!(detect_path(".gitignore"), "ignore");
        assert_eq!(detect_path(".editorconfig"), "ini");
        assert_eq!(detect_path("notes"), PLAIN_TEXT);
        assert_eq!(detect(None, None, &Associations::default()), PLAIN_TEXT);
    }

    #[test]
    fn shebangs_identify_extensionless_scripts() {
        let none = Associations::default();
        let script = Some(Path::new("bin/run"));
        assert_eq!(detect(script, Some("#!/bin/bash"), &none), "shell");
        assert_eq!(
            detect(script, Some("#!/usr/bin/env python3"), &none),
            "python"
        );
        assert_eq!(
            detect(script, Some("#!/usr/bin/env -S node --flag"), &none),
            "javascript"
        );
        assert_eq!(
            detect(script, Some("#!/usr/bin/python3.12"), &none),
            "python"
        );
        assert_eq!(detect(script, Some("plain"), &none), PLAIN_TEXT);
        // Extensions win over shebangs.
        assert_eq!(
            detect(Some(Path::new("x.rs")), Some("#!/bin/sh"), &none),
            "rust"
        );
        assert_eq!(detect(None, Some("#!/bin/sh"), &none), "shell");
    }

    #[test]
    fn user_associations_override_built_in_rules() {
        let (associations, problems) = Associations::new([
            ("*.h", "cpp"),
            ("Jenkinsfile", "Java"),
            ("config/*.conf", "yaml"),
            ("*.x", "klingon"),
            ("{bad", "rust"),
        ]);
        assert_eq!(problems.len(), 2);
        let detect = |path: &str| detect(Some(&PathBuf::from(path)), None, &associations);
        assert_eq!(detect("include/a.h"), "cpp");
        assert_eq!(detect("Jenkinsfile"), "java");
        assert_eq!(detect("config/app.conf"), "yaml");
        assert_eq!(detect("other/app.conf"), "ini");
    }

    #[test]
    fn registry_is_consistent() {
        for language in LANGUAGES {
            assert_eq!(by_id(language.id), Some(language), "{}", language.id);
            if let Some((open, close)) = language.block_comment {
                assert!(!open.is_empty() && !close.is_empty());
            }
        }
        let mut extensions: Vec<_> = LANGUAGES.iter().flat_map(|l| l.extensions).collect();
        let count = extensions.len();
        extensions.sort();
        extensions.dedup();
        assert_eq!(count, extensions.len(), "extensions must be unique");
        assert_eq!(get("missing").id, PLAIN_TEXT);
    }
}
