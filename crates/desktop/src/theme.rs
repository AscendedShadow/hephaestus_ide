use std::{cell::Cell, sync::Arc};

use gpui::{App, Font, Rgba, Window, font, rgb};
use gpui_component::{Theme, ThemeMode, highlighter::SyntaxColors};
use ide_core::git::Change;

use crate::syntax::Token;

thread_local! {
    static MODE: Cell<ThemeMode> = const { Cell::new(ThemeMode::Dark) };
}

pub fn mode() -> ThemeMode {
    MODE.get()
}

pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    MODE.set(mode);
    Theme::change(mode, window, cx);
    let theme = Theme::global_mut(cx);
    let mut highlight = (*theme.highlight_theme).clone();
    highlight.style.syntax = syntax_colors();
    theme.highlight_theme = Arc::new(highlight);
}

fn pick(dark: u32, light: u32) -> Rgba {
    rgb(if mode().is_dark() { dark } else { light })
}

pub fn background() -> Rgba {
    pick(0x1e1f22, 0xffffff)
}

pub fn panel() -> Rgba {
    pick(0x2b2d30, 0xf7f8fa)
}

pub fn border() -> Rgba {
    pick(0x393b40, 0xebecf0)
}

pub fn text() -> Rgba {
    pick(0xdfe1e5, 0x1f2023)
}

pub fn muted() -> Rgba {
    pick(0x9da0a8, 0x6c707e)
}

pub fn accent() -> Rgba {
    pick(0x8aabff, 0x3574f0)
}

pub fn selection() -> Rgba {
    pick(0x214283, 0xa6d2ff)
}

pub fn git_change(change: Change) -> Rgba {
    match change {
        Change::Added => pick(0x73bd79, 0x3f8f3a),
        Change::Untracked => pick(0xd1675a, 0xb93b2c),
        Change::Deleted => pick(0x868a91, 0x6c707e),
        Change::Conflicted => pick(0xe5b95c, 0x9d6c00),
        Change::Modified | Change::Renamed | Change::Copied | Change::TypeChanged => {
            pick(0x70aeff, 0x1a64d6)
        }
    }
}

pub fn diff_added() -> Rgba {
    pick(0x294436, 0xdcf5dc)
}

pub fn diff_removed() -> Rgba {
    pick(0x4c2b2d, 0xfbe0e0)
}

pub fn error() -> Rgba {
    pick(0xf06c6c, 0xc4432f)
}

pub fn syntax(token: Token) -> Rgba {
    match token {
        Token::Keyword => pick(0xcf8e6d, 0xb3541e),
        Token::Declaration => pick(0x56a8f5, 0x1f5fd1),
        Token::Call => pick(0xe5c07b, 0x8a6400),
        Token::Name => pick(0xc39ae8, 0x7b3fc0),
        Token::Type => pick(0x4ec9b0, 0x08806f),
        Token::Constant => pick(0xee92b0, 0xc0306a),
        Token::String => pick(0x6aab73, 0x3d8a2e),
        Token::Comment => pick(0x7a7e85, 0x8c8c8c),
        Token::DocComment => pick(0x5f826b, 0x53785d),
        Token::Attribute => pick(0xb3ae60, 0x6f7520),
    }
}

pub fn syntax_colors() -> SyntaxColors {
    let styles = Token::ALL
        .into_iter()
        .map(|token| {
            let key = match token {
                Token::DocComment => "comment_doc",
                _ => token.capture(),
            };
            let color = format!("#{:06x}", hex(syntax(token)));
            (key.to_string(), serde_json::json!({ "color": color }))
        })
        .collect();
    serde_json::from_value(serde_json::Value::Object(styles))
        .expect("syntax colors are valid theme styles")
}

pub fn hex(color: Rgba) -> u32 {
    let channel = |value: f32| (value * 255.).round() as u32;
    channel(color.r) << 16 | channel(color.g) << 8 | channel(color.b)
}

pub fn monospace_font() -> Font {
    font(if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "monospace"
    })
}

pub fn terminal_palette() -> terminal::Palette {
    let defaults = terminal::Palette::default();
    terminal::Palette {
        foreground: hex(text()),
        background: hex(background()),
        cursor: hex(text()),
        ansi: if mode().is_dark() {
            defaults.ansi
        } else {
            [
                0x383a42, 0xd13c3c, 0x3f8f3a, 0x9d6c00, 0x3574f0, 0xa626a4, 0x0e7d91, 0x9a9ca4,
                0x6c707e, 0xe45649, 0x50a14f, 0xc18401, 0x4078f2, 0xb750b5, 0x0184bc, 0x383a42,
            ]
        },
    }
}
