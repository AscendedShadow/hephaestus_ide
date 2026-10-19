use std::fs;

use gpui::{AppContext as _, TestAppContext};

use super::*;
use crate::settings;

const SOURCE: &str = r#"/// Adds one to `value`.
///
/// Returns the sum.
#[inline]
pub fn increment(value: i32) -> i32 {
    let doubled = value * 2;
    doubled + increment(value)
}
"#;

fn hover_text(source: &str, word: &str, occurrence: usize) -> String {
    let offset = source
        .match_indices(word)
        .map(|(offset, _)| offset)
        .nth(occurrence)
        .unwrap();
    let hover = declaration_hover("rust", source, offset + 1, None, Some(Path::new("lib.rs")))
        .expect("a declaration");
    match hover.contents {
        HoverContents::Markup(markup) => markup.value,
        other => panic!("{other:?}"),
    }
}

#[test]
fn hovering_a_call_shows_its_signature_docs_and_location() {
    let text = hover_text(SOURCE, "increment", 1);
    assert_eq!(
        text,
        "```rust\npub fn increment(value: i32) -> i32\n```\n\nAdds one to `value`.\n\nReturns the sum.\n\n`lib.rs:5`"
    );
}

#[test]
fn hovering_a_local_shows_the_declaration_without_docs() {
    assert_eq!(
        hover_text(SOURCE, "doubled", 1),
        "```rust\nlet doubled = value * 2;\n```\n\n`lib.rs:6`"
    );
    assert_eq!(
        hover_text(SOURCE, "value", 2),
        "```rust\nvalue: i32\n```\n\n`lib.rs:5`"
    );
}

#[test]
fn hovering_something_undeclared_shows_nothing() {
    assert_eq!(
        declaration_hover("rust", SOURCE, SOURCE.find("i32").unwrap(), None, None),
        None
    );
    assert_eq!(
        declaration_hover("plain_text", "value", 1, None, None),
        None
    );
}

#[test]
fn hovering_reaches_declarations_in_other_files() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    fs::write(
        root.join("shapes.rs"),
        "// A round shape.\npub struct Circle {\n    radius: f64,\n}\n",
    )
    .unwrap();
    let source = "use crate::shapes::Circle;\nfn area(circle: Circle) {}\n";
    let origin = root.join("main.rs");
    fs::write(&origin, source).unwrap();

    let offset = source.rfind("Circle").unwrap() + 1;
    let hover = declaration_hover("rust", source, offset, Some(&root), Some(&origin)).unwrap();
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("expected markdown");
    };
    assert_eq!(
        markup.value,
        "```rust\npub struct Circle\n```\n\nA round shape.\n\n`shapes.rs:2`"
    );
}

#[test]
fn positions_count_utf16_units_in_the_line() {
    let text = "let greeting = \"héllo 👋\";\nlet next = 1;\n";
    assert_eq!(position_at(text, 0), Position::new(0, 0));
    let offset = text.find("👋").unwrap();
    assert_eq!(position_at(text, offset), Position::new(0, 22));
    assert_eq!(position_at(text, offset + 4), Position::new(0, 24));
    let line = text.find("let next").unwrap();
    assert_eq!(position_at(text, line + 4), Position::new(1, 4));
    assert_eq!(position_at(text, text.len() + 99), Position::new(2, 0));
}

#[gpui::test]
fn servers_start_only_for_configured_languages_inside_a_project(cx: &mut TestAppContext) {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().canonicalize().unwrap();
    let servers = cx.update(|cx| cx.new(|_| Servers::default()));
    let configs = vec![settings::LanguageServer {
        languages: vec!["rust".into()],
        program: "hephaestus-language-server-that-does-not-exist".into(),
        ..Default::default()
    }];

    servers.update(cx, |servers, cx| {
        servers.set_configs(configs.clone(), cx);
        assert_eq!(servers.state("go"), None);
        assert_eq!(
            servers.state("rust"),
            Some((configs[0].program.clone(), State::Starting))
        );

        servers.sync(&root.join("main.rs"), "rust", "fn main() {}", cx);
        assert!(servers.documents.is_empty(), "no project root yet");

        servers.set_root(Some(root.clone()), cx);
        servers.sync(&root.join("main.rs"), "rust", "fn main() {}", cx);
        assert!(servers.documents.is_empty(), "the program cannot be run");
        assert!(servers.hover(&root, "rust", Position::new(0, 0)).is_none());
        assert!(matches!(servers.state("rust"), Some((_, State::Failed(_)))));

        servers.sync(&root.join("main.rs"), "rust", "fn main() {}", cx);
        assert_eq!(servers.failures.len(), 1, "a failed program is not retried");
        servers.stop(cx);
        assert!(servers.failures.is_empty(), "restarting clears failures");
    });
}
