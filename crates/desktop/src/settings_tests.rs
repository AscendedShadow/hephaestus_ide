use gpui::{TestAppContext, rgb, rgba};
use gpui_component::{Theme, ThemeMode};

use super::*;
use crate::{
    commands::{Command, Keys},
    syntax::Token,
    theme::{Color, Hex},
};

#[test]
fn defaults_round_trip_through_the_settings_file() {
    let defaults = Settings::defaults();
    let json = defaults.to_json();
    assert!(json.contains("\"toggle_terminal\": \"ctrl-`\""), "{json}");
    assert!(json.contains("\"panel\": \"#16171b\""), "{json}");
    assert!(json.contains("\"keyword\": \"#c792ea\""), "{json}");
    assert_eq!(Settings::parse(&json), Ok(defaults));
}

#[test]
fn partial_settings_accept_single_keys_key_lists_and_hex_colors() {
    let settings = Settings::parse(
        r##"{
            "keybindings": {
                "save_file": "alt-s",
                "toggle_terminal": ["ctrl-j", "ctrl-`"],
                "toggle_fold": []
            },
            "theme": {
                "dark": {
                    "colors": { "panel": "#102030" },
                    "syntax": { "doc_comment": "#ff000080" }
                }
            }
        }"##,
    )
    .unwrap();
    let keys = |command| settings.keybindings.get(&command).cloned();
    assert_eq!(keys(Command::SaveFile), Some(Keys::One("alt-s".into())));
    assert_eq!(
        keys(Command::ToggleTerminal),
        Some(Keys::Many(vec!["ctrl-j".into(), "ctrl-`".into()]))
    );
    assert_eq!(keys(Command::ToggleFold), Some(Keys::Many(Vec::new())));
    assert_eq!(keys(Command::NewFile), None);
    let dark = &settings.theme.dark;
    assert_eq!(dark.colors.get(&Color::Panel), Some(&Hex(rgb(0x102030))));
    assert_eq!(
        dark.syntax.get(&Token::DocComment),
        Some(&Hex(rgba(0xff000080)))
    );
    assert_eq!(settings.theme.light, theme::Palette::default());
    assert!(
        serde_json::to_string(&Hex(rgba(0xff000080)))
            .unwrap()
            .contains("#ff000080")
    );
}

#[test]
fn unknown_names_and_bad_colors_are_reported() {
    let error = |source: &str| Settings::parse(source).unwrap_err();
    assert!(error(r#"{ "keybindings": { "launch": "ctrl-l" } }"#).contains("launch"));
    assert!(
        error(r##"{ "theme": { "dark": { "colors": { "pannel": "#000000" } } } }"##)
            .contains("pannel")
    );
    assert!(
        error(r#"{ "theme": { "dark": { "syntax": { "keyword": "purple" } } } }"#)
            .contains("line 1")
    );
    assert!(error(r#"{ "colours": {} }"#).contains("colours"));
    assert!(error("{").starts_with(FILE_NAME));
}

#[test]
fn a_missing_file_loads_the_defaults() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("nested").join(FILE_NAME);
    assert_eq!(Settings::load(&path), Ok(Settings::default()));
    create_if_missing(&path).unwrap();
    assert_eq!(Settings::load(&path), Ok(Settings::defaults()));
    std::fs::write(&path, "{}").unwrap();
    create_if_missing(&path).unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "{}");
}

#[gpui::test]
fn applying_settings_recolors_the_theme_and_reports_bad_keys(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        theme::set_mode(ThemeMode::Dark, None, cx);
        commands::init(cx);
        let settings = Settings::parse(
            r##"{
                "keybindings": { "new_file": ["alt-n", "ctrl-nope-n"] },
                "theme": {
                    "dark": {
                        "colors": { "panel": "#102030" },
                        "syntax": { "keyword": "#ff0000" }
                    }
                }
            }"##,
        )
        .unwrap();
        let problems = settings.apply(None, cx);
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].starts_with("new_file: "), "{problems:?}");

        assert_eq!(theme::panel(), rgb(0x102030));
        assert_eq!(Theme::global(cx).colors.sidebar, rgb(0x102030).into());
        assert_eq!(theme::syntax(Token::Keyword), rgb(0xff0000));
        let syntax = &Theme::global(cx).highlight_theme.style.syntax;
        let keyword = syntax.style("keyword").and_then(|style| style.color);
        assert_eq!(keyword, Some(rgb(0xff0000).into()));
        assert_eq!(theme::syntax(Token::String), rgb(0xa5d68b));

        theme::set_mode(ThemeMode::Light, None, cx);
        assert_eq!(theme::panel(), rgb(0xf6f7f9));
        assert_eq!(theme::syntax(Token::Keyword), rgb(0x8e3fc9));
    });
}
