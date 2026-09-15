use std::{ffi::OsStr, path::Path};

use gpui_component::highlighter::{LanguageConfig, LanguageRegistry};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

pub fn init() {
    let registry = LanguageRegistry::singleton();
    let languages = [
        ("c", tree_sitter_c::LANGUAGE, C_HIGHLIGHTS),
        ("c_sharp", tree_sitter_c_sharp::LANGUAGE, C_SHARP_HIGHLIGHTS),
        ("go", tree_sitter_go::LANGUAGE, GO_HIGHLIGHTS),
        ("java", tree_sitter_java::LANGUAGE, JAVA_HIGHLIGHTS),
        (
            "javascript",
            tree_sitter_javascript::LANGUAGE,
            JAVASCRIPT_HIGHLIGHTS,
        ),
        ("jsx", tree_sitter_javascript::LANGUAGE, JSX_HIGHLIGHTS),
        ("rust", tree_sitter_rust::LANGUAGE, RUST_HIGHLIGHTS),
        (
            "typescript",
            tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
            TYPESCRIPT_HIGHLIGHTS,
        ),
        ("tsx", tree_sitter_typescript::LANGUAGE_TSX, TSX_HIGHLIGHTS),
        ("zig", tree_sitter_zig::LANGUAGE, ZIG_HIGHLIGHTS),
    ];
    for (name, grammar, highlights) in languages {
        registry.register(
            name,
            &LanguageConfig::new(name, grammar.into(), Vec::new(), highlights, "", ""),
        );
    }
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
