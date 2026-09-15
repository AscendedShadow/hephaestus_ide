use std::{ffi::OsStr, path::Path};

use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};
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

const HIGHLIGHTS: [(&str, &str); 10] = [
    ("c", C_HIGHLIGHTS),
    ("c_sharp", C_SHARP_HIGHLIGHTS),
    ("go", GO_HIGHLIGHTS),
    ("java", JAVA_HIGHLIGHTS),
    ("javascript", JAVASCRIPT_HIGHLIGHTS),
    ("jsx", JSX_HIGHLIGHTS),
    ("rust", RUST_HIGHLIGHTS),
    ("typescript", TYPESCRIPT_HIGHLIGHTS),
    ("tsx", TSX_HIGHLIGHTS),
    ("zig", ZIG_HIGHLIGHTS),
];

pub fn init() {
    let registry = LanguageRegistry::singleton();
    for (name, highlights) in HIGHLIGHTS {
        if let Some(grammar) = grammar(name) {
            registry.register(
                name,
                &LanguageConfig::new(name, grammar, Vec::new(), highlights, "", ""),
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
        _ => return None,
    };
    Some(grammar.into())
}

pub fn language(path: Option<&Path>) -> &'static str {
    match path.and_then(Path::extension).and_then(OsStr::to_str) {
        Some("c" | "h") => "c",
        Some("cs") => "c_sharp",
        Some("go") => "go",
        Some("java") => "java",
        Some("js" | "mjs" | "cjs") => "javascript",
        Some("jsx") => "jsx",
        Some("rs") => "rust",
        Some("ts" | "mts" | "cts") => "typescript",
        Some("tsx") => "tsx",
        Some("zig") => "zig",
        _ => "plain_text",
    }
}
