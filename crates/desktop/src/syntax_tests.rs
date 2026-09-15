use std::collections::HashSet;

use gpui::{Rgba, TestAppContext};
use gpui_component::{
    Theme, ThemeMode,
    highlighter::{HighlightTheme, HighlightThemeStyle, SyntaxHighlighter},
};
use ropey::Rope;

use super::*;
use crate::theme;

use Token::*;

fn byte_tokens(language: &str, source: &str) -> Vec<Option<Token>> {
    init();
    let theme = HighlightTheme {
        name: "test".into(),
        appearance: theme::mode(),
        style: HighlightThemeStyle {
            syntax: theme::syntax_colors(),
            ..Default::default()
        },
    };
    let mut highlighter = SyntaxHighlighter::new(language);
    highlighter.update(None, &Rope::from(source));
    let mut tokens = vec![None; source.len()];
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let line_range = offset..offset + line.len();
        for (range, style) in highlighter.styles(&line_range, &theme) {
            let token = style.color.map(|color| {
                let color = theme::hex(Rgba::from(color));
                *Token::ALL
                    .iter()
                    .find(|&&token| theme::hex(theme::syntax(token)) == color)
                    .unwrap_or_else(|| panic!("{color:06x} is not a token color"))
            });
            tokens[range].fill(token);
        }
        offset = line_range.end;
    }
    tokens
}

fn find_word(source: &str, word: &str, from: usize) -> usize {
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    source[from..]
        .match_indices(word)
        .map(|(ix, _)| from + ix)
        .find(|&start| {
            let before = source[..start].chars().next_back();
            let after = source[start + word.len()..].chars().next();
            !(word.starts_with(is_name) && before.is_some_and(is_name))
                && !(word.ends_with(is_name) && after.is_some_and(is_name))
        })
        .unwrap_or_else(|| panic!("{word:?} is not in the source after byte {from}"))
}

#[track_caller]
fn assert_tokens(language: &str, source: &str, expected: &[(&str, Option<Token>)]) {
    let tokens = byte_tokens(language, source);
    let mut from = 0;
    let actual: Vec<_> = expected
        .iter()
        .map(|&(word, _)| {
            let start = find_word(source, word, from);
            from = start + word.len();
            let word_tokens = &tokens[start..from];
            let token = if word_tokens.iter().all(|token| *token == word_tokens[0]) {
                format!("{:?}", word_tokens[0])
            } else {
                format!("mixed {word_tokens:?}")
            };
            (word, token)
        })
        .collect();
    let expected: Vec<_> = expected
        .iter()
        .map(|&(word, token)| (word, format!("{token:?}")))
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn declarations_calls_names_and_types_are_told_apart() {
    let source = r#"
pub struct Totals<T> {
    count: usize,
}

const LIMIT: u32 = 10;

fn total(items: &[Item], limit: Option<u32>) -> Totals<u64> {
    let mut sum = 0;
    for item in items.iter() {
        sum += weight(item) + item.size;
    }
    let map = HashMap::<String, u32>::new();
    let text = std::fs::read_to_string(Path::new("a")).ok();
    Totals { count: sum }
}

trait Shape {
    fn area(&self) -> f64;
}

enum Tone {
    Light,
}
"#;
    assert_tokens(
        "rust",
        source,
        &[
            ("pub", Some(Keyword)),
            ("struct", Some(Keyword)),
            ("Totals", Some(Declaration)),
            ("T", Some(Type)),
            ("count", Some(Name)),
            ("usize", Some(Type)),
            ("const", Some(Keyword)),
            ("LIMIT", Some(Declaration)),
            ("u32", Some(Type)),
            ("10", Some(Constant)),
            ("fn", Some(Keyword)),
            ("total", Some(Declaration)),
            ("items", Some(Name)),
            ("Item", Some(Type)),
            ("limit", Some(Name)),
            ("Option", Some(Type)),
            ("->", None),
            ("Totals", Some(Type)),
            ("u64", Some(Type)),
            ("let", Some(Keyword)),
            ("mut", Some(Keyword)),
            ("sum", Some(Name)),
            ("=", None),
            ("0", Some(Constant)),
            ("item", Some(Name)),
            ("items", Some(Name)),
            ("iter", Some(Call)),
            ("sum", Some(Name)),
            ("+=", None),
            ("weight", Some(Call)),
            ("item", Some(Name)),
            ("+", None),
            ("item", Some(Name)),
            ("size", Some(Name)),
            ("map", Some(Name)),
            ("HashMap", Some(Type)),
            ("String", Some(Type)),
            ("new", Some(Call)),
            ("text", Some(Name)),
            ("std", Some(Type)),
            ("fs", Some(Type)),
            ("read_to_string", Some(Call)),
            ("Path", Some(Type)),
            ("new", Some(Call)),
            ("\"a\"", Some(String)),
            ("ok", Some(Call)),
            ("Totals", Some(Type)),
            ("count", Some(Name)),
            ("sum", Some(Name)),
            ("Shape", Some(Declaration)),
            ("area", Some(Declaration)),
            ("self", Some(Keyword)),
            ("f64", Some(Type)),
            ("Tone", Some(Declaration)),
            ("Light", Some(Declaration)),
        ],
    );
}

#[test]
fn macros_attributes_comments_and_literals() {
    let source = r#"
//! The crate.
use std::collections::{BTreeMap, HashMap};

/// Doc for `run`.
#[derive(Debug, Clone)]
#[cfg(test)]
fn run<'a>(name: &'a str) {
    // Plain comment.
    println!("{} {}", name, describe(name.len()));
    let first = Some(LIMIT);
    if first == None {
        return;
    }
    let text = vec![Tone::Light, Tone::new(), format!("{}", x), vec![x]];
    'outer: loop {
        break 'outer;
    }
    let letter = 'x';
    let yes = true;
}
"#;
    assert_tokens(
        "rust",
        source,
        &[
            ("The", Some(DocComment)),
            ("use", Some(Keyword)),
            ("std", Some(Type)),
            ("collections", Some(Type)),
            ("BTreeMap", Some(Name)),
            ("HashMap", Some(Name)),
            ("Doc", Some(DocComment)),
            ("#[", Some(Attribute)),
            ("derive", Some(Attribute)),
            ("Debug", Some(Attribute)),
            ("Clone", Some(Attribute)),
            ("cfg", Some(Attribute)),
            ("test", Some(Attribute)),
            ("run", Some(Declaration)),
            ("'a", Some(Keyword)),
            ("name", Some(Name)),
            ("'a", Some(Keyword)),
            ("str", Some(Type)),
            ("Plain", Some(Comment)),
            ("println!", Some(Call)),
            ("\"{} {}\"", Some(String)),
            ("name", Some(Name)),
            ("describe", Some(Call)),
            ("name", Some(Name)),
            ("len", Some(Call)),
            ("Some", Some(Call)),
            ("LIMIT", Some(Name)),
            ("first", Some(Name)),
            ("==", None),
            ("None", Some(Name)),
            ("return", Some(Keyword)),
            ("vec!", Some(Call)),
            ("Tone", Some(Type)),
            ("Light", Some(Name)),
            ("Tone", Some(Type)),
            ("new", Some(Call)),
            ("format!", Some(Call)),
            ("x", Some(Name)),
            ("vec!", Some(Call)),
            ("x", Some(Name)),
            ("'outer", Some(Keyword)),
            ("loop", Some(Keyword)),
            ("'outer", Some(Keyword)),
            ("'x'", Some(String)),
            ("true", Some(Constant)),
        ],
    );
}

#[test]
fn file_extensions_select_their_language() {
    assert_eq!(language(Some(Path::new("src/main.rs"))), "rust");
    assert_eq!(language(Some(Path::new("src/main.c"))), "c");
    assert_eq!(language(Some(Path::new("include/main.h"))), "c");
    assert_eq!(language(Some(Path::new("Program.cs"))), "c_sharp");
    assert_eq!(language(Some(Path::new("main.go"))), "go");
    assert_eq!(language(Some(Path::new("Main.java"))), "java");
    assert_eq!(language(Some(Path::new("index.js"))), "javascript");
    assert_eq!(language(Some(Path::new("index.jsx"))), "jsx");
    assert_eq!(language(Some(Path::new("index.mjs"))), "javascript");
    assert_eq!(language(Some(Path::new("index.cjs"))), "javascript");
    assert_eq!(language(Some(Path::new("index.ts"))), "typescript");
    assert_eq!(language(Some(Path::new("index.mts"))), "typescript");
    assert_eq!(language(Some(Path::new("index.cts"))), "typescript");
    assert_eq!(language(Some(Path::new("view.tsx"))), "tsx");
    assert_eq!(language(Some(Path::new("main.zig"))), "zig");
    assert_eq!(language(Some(Path::new("notes.txt"))), "plain_text");
    assert_eq!(language(Some(Path::new("Makefile"))), "plain_text");
    assert_eq!(language(None), "plain_text");
}

#[test]
fn configured_languages_have_semantic_highlights() {
    let cases = [
        ("c", "int add(int n) { return use(n) + 1; }", "add", "use"),
        (
            "c_sharp",
            "class Box { int Add(int n) { return Use(n) + 1; } }",
            "Add",
            "Use",
        ),
        (
            "go",
            "func add(n int) int { return use(n) + 1 }",
            "add",
            "use",
        ),
        (
            "java",
            "class Box { int add(int n) { return use(n) + 1; } }",
            "add",
            "use",
        ),
        (
            "javascript",
            "function add(n) { return use(n) + 1; }",
            "add",
            "use",
        ),
        (
            "jsx",
            "function View() { return <Box count={1} />; }",
            "View",
            "Box",
        ),
        (
            "typescript",
            "function add(n: number): number { return use(n) + 1; }",
            "add",
            "use",
        ),
        (
            "tsx",
            "function View() { return <Box count={1} />; }",
            "View",
            "Box",
        ),
        (
            "zig",
            "fn add(n: i32) i32 { return use(n) + 1; }",
            "add",
            "use",
        ),
    ];

    for (language, source, declaration, call) in cases {
        let config = LanguageRegistry::singleton().language(language).unwrap();
        tree_sitter::Query::new(&config.language, &config.highlights)
            .unwrap_or_else(|error| panic!("invalid {language} highlights: {error}"));
        assert_tokens(
            language,
            source,
            &[
                (declaration, Some(Declaration)),
                (
                    call,
                    Some(if matches!(language, "jsx" | "tsx") {
                        Type
                    } else {
                        Call
                    }),
                ),
                ("1", Some(Constant)),
            ],
        );
    }
}

#[gpui::test]
fn each_mode_gives_every_kind_of_token_its_own_color(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        for mode in [ThemeMode::Dark, ThemeMode::Light] {
            theme::set_mode(mode, None, cx);
            let syntax = &Theme::global(cx).highlight_theme.style.syntax;
            for token in Token::ALL {
                let color = syntax.style(token.capture()).and_then(|style| style.color);
                assert_eq!(
                    color,
                    Some(theme::syntax(token).into()),
                    "{mode:?} {token:?}"
                );
            }
            let mut colors: HashSet<_> = Token::ALL
                .iter()
                .map(|&token| theme::hex(theme::syntax(token)))
                .collect();
            assert_eq!(colors.len(), Token::ALL.len(), "{mode:?}");
            assert!(colors.insert(theme::hex(theme::text())), "{mode:?}");
        }
    });
}
