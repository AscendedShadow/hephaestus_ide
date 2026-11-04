use std::{
    path::Path,
    sync::{LazyLock, RwLock},
};

use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};
use ide_core::language::{self as registry, Associations};
use serde::{Deserialize, Serialize};

#[cfg(test)]
#[path = "syntax_tests.rs"]
mod tests;

const C_HIGHLIGHTS: &str = include_str!("syntax/c.scm");
const C_SHARP_HIGHLIGHTS: &str = include_str!("syntax/c_sharp.scm");
const GO_HIGHLIGHTS: &str = include_str!("syntax/go.scm");
const JAVA_HIGHLIGHTS: &str = include_str!("syntax/java.scm");
const JAVASCRIPT_HIGHLIGHTS: &str = include_str!("syntax/javascript.scm");
const JSX_HIGHLIGHTS: &str = concat!(
    include_str!("syntax/javascript.scm"),
    include_str!("syntax/tsx.scm")
);
const RUST_HIGHLIGHTS: &str = include_str!("syntax/rust.scm");
const TYPESCRIPT_HIGHLIGHTS: &str = concat!(
    include_str!("syntax/javascript.scm"),
    include_str!("syntax/typescript.scm")
);
const TSX_HIGHLIGHTS: &str = concat!(
    include_str!("syntax/javascript.scm"),
    include_str!("syntax/typescript.scm"),
    include_str!("syntax/tsx.scm")
);
const ZIG_HIGHLIGHTS: &str = include_str!("syntax/zig.scm");
const MARKDOWN_HIGHLIGHTS: &str = include_str!(
    "../../../vendor/gpui-component/src/highlighter/languages/markdown/highlights.scm"
);
const JSON_HIGHLIGHTS: &str =
    include_str!("../../../vendor/gpui-component/src/highlighter/languages/json/highlights.scm");

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Token {
    Keyword,
    Declaration,
    Call,
    Name,
    Type,
    Constant,
    String,
    Comment,
    DocComment,
    Attribute,
}

impl Token {
    pub const ALL: [Self; 10] = [
        Self::Keyword,
        Self::Declaration,
        Self::Call,
        Self::Name,
        Self::Type,
        Self::Constant,
        Self::String,
        Self::Comment,
        Self::DocComment,
        Self::Attribute,
    ];

    pub fn capture(self) -> &'static str {
        match self {
            Self::Keyword => "keyword",
            Self::Declaration => "title",
            Self::Call => "function",
            Self::Name => "variable",
            Self::Type => "type",
            Self::Constant => "constant",
            Self::String => "string",
            Self::Comment => "comment",
            Self::DocComment => "comment.doc",
            Self::Attribute => "attribute",
        }
    }
}

fn highlights(language: &str) -> Option<String> {
    Some(match language {
        "c" => C_HIGHLIGHTS.into(),
        "cpp" => format!("{C_HIGHLIGHTS}\n{}", tree_sitter_cpp::HIGHLIGHT_QUERY),
        "c_sharp" => C_SHARP_HIGHLIGHTS.into(),
        "go" => GO_HIGHLIGHTS.into(),
        "java" => JAVA_HIGHLIGHTS.into(),
        "javascript" => JAVASCRIPT_HIGHLIGHTS.into(),
        "jsx" => JSX_HIGHLIGHTS.into(),
        "rust" => RUST_HIGHLIGHTS.into(),
        "typescript" => TYPESCRIPT_HIGHLIGHTS.into(),
        "tsx" => TSX_HIGHLIGHTS.into(),
        "zig" => ZIG_HIGHLIGHTS.into(),
        "python" => tree_sitter_python::HIGHLIGHTS_QUERY.into(),
        "shell" => tree_sitter_bash::HIGHLIGHT_QUERY.into(),
        "json" | "jsonc" => JSON_HIGHLIGHTS.into(),
        "toml" => tree_sitter_toml_ng::HIGHLIGHTS_QUERY.into(),
        "yaml" => tree_sitter_yaml::HIGHLIGHTS_QUERY.into(),
        "markdown" => MARKDOWN_HIGHLIGHTS.into(),
        "html" => tree_sitter_html::HIGHLIGHTS_QUERY.into(),
        "css" => tree_sitter_css::HIGHLIGHTS_QUERY.into(),
        "makefile" => tree_sitter_make::HIGHLIGHTS_QUERY.into(),
        _ => return None,
    })
}

static ASSOCIATIONS: LazyLock<RwLock<Associations>> =
    LazyLock::new(|| RwLock::new(Associations::default()));

/// Install user file associations (from settings); returns problems with invalid rules.
pub fn set_associations<'a>(rules: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<String> {
    let (associations, problems) = Associations::new(rules);
    if let Ok(mut current) = ASSOCIATIONS.write() {
        *current = associations;
    }
    problems
}

pub fn init() {
    let registry = LanguageRegistry::singleton();
    for language in registry::all() {
        let name = language.id;
        if let (Some(grammar), Some(highlights)) = (grammar(name), highlights(name)) {
            registry.register(
                name,
                &LanguageConfig::new(name, grammar, Vec::new(), &highlights, "", ""),
            );
        }
    }
}

pub fn grammar(language: &str) -> Option<tree_sitter::Language> {
    let grammar = match language {
        "c" => tree_sitter_c::LANGUAGE,
        "c_sharp" => tree_sitter_c_sharp::LANGUAGE,
        "go" => tree_sitter_go::LANGUAGE,
        "java" => tree_sitter_java::LANGUAGE,
        "javascript" | "jsx" => tree_sitter_javascript::LANGUAGE,
        "rust" => tree_sitter_rust::LANGUAGE,
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        "tsx" => tree_sitter_typescript::LANGUAGE_TSX,
        "zig" => tree_sitter_zig::LANGUAGE,
        "cpp" => tree_sitter_cpp::LANGUAGE,
        "python" => tree_sitter_python::LANGUAGE,
        "shell" => tree_sitter_bash::LANGUAGE,
        "json" | "jsonc" => tree_sitter_json::LANGUAGE,
        "toml" => tree_sitter_toml_ng::LANGUAGE,
        "yaml" => tree_sitter_yaml::LANGUAGE,
        "markdown" => tree_sitter_md::LANGUAGE,
        "html" => tree_sitter_html::LANGUAGE,
        "css" => tree_sitter_css::LANGUAGE,
        "makefile" => tree_sitter_make::LANGUAGE,
        _ => return None,
    };
    Some(grammar.into())
}

/// Detect a file's language mode from its path (user associations, file name and
/// extension). Use [`detect`] when the first line is available for shebang detection.
pub fn language(path: Option<&Path>) -> &'static str {
    detect(path, None)
}

/// Detect a language mode from the path and the document's first line.
pub fn detect(path: Option<&Path>, first_line: Option<&str>) -> &'static str {
    let associations = ASSOCIATIONS.read();
    match associations {
        Ok(associations) => registry::detect(path, first_line, &associations),
        Err(_) => registry::detect(path, first_line, &Associations::default()),
    }
}

/// Whether a syntax grammar is available for a language mode.
pub fn has_grammar(language: &str) -> bool {
    grammar(language).is_some() && highlights(language).is_some()
}
