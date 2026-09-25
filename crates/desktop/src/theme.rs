use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    sync::Arc,
};

use gpui::{App, Font, FontFallbacks, Hsla, Rgba, Window, font, px, rgb, rgba};
use gpui_component::{Theme, ThemeMode, highlighter::SyntaxColors};
use ide_core::git::Change;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{file_icons::FileKind, syntax::Token};

thread_local! {
    static MODE: Cell<ThemeMode> = const { Cell::new(ThemeMode::Dark) };
    static MONO_FAMILY: Cell<&'static str> = const { Cell::new(FALLBACK_MONO) };
    static THEMES: RefCell<Themes> = RefCell::new(Themes::default());
}

const FALLBACK_MONO: &str = if cfg!(target_os = "windows") {
    "Consolas"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "monospace"
};

const PREFERRED_MONO: [&str; 5] = [
    "Cascadia Mono",
    "JetBrains Mono",
    "SF Mono",
    "Menlo",
    "DejaVu Sans Mono",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Color {
    Background,
    Panel,
    Chrome,
    Elevated,
    Border,
    BorderStrong,
    Hover,
    ActiveRow,
    ActiveLine,
    Text,
    Muted,
    Subtle,
    Accent,
    Selection,
    Error,
    GitAdded,
    GitModified,
    GitUntracked,
    GitDeleted,
    GitConflicted,
    DiffAdded,
    DiffRemoved,
    DiffHunk,
}

impl Color {
    pub const ALL: [Self; 23] = [
        Self::Background,
        Self::Panel,
        Self::Chrome,
        Self::Elevated,
        Self::Border,
        Self::BorderStrong,
        Self::Hover,
        Self::ActiveRow,
        Self::ActiveLine,
        Self::Text,
        Self::Muted,
        Self::Subtle,
        Self::Accent,
        Self::Selection,
        Self::Error,
        Self::GitAdded,
        Self::GitModified,
        Self::GitUntracked,
        Self::GitDeleted,
        Self::GitConflicted,
        Self::DiffAdded,
        Self::DiffRemoved,
        Self::DiffHunk,
    ];

    fn defaults(self) -> (u32, u32) {
        match self {
            Self::Background => (0x1b1c21, 0xffffff),
            Self::Panel => (0x16171b, 0xf6f7f9),
            Self::Chrome => (0x121317, 0xeef0f3),
            Self::Elevated => (0x222329, 0xffffff),
            Self::Border => (0x26282e, 0xe3e5ea),
            Self::BorderStrong => (0x33363d, 0xd3d6dc),
            Self::Hover => (0x23252b, 0xe8eaee),
            Self::ActiveRow => (0x243150, 0xdde7ff),
            Self::ActiveLine => (0x202127, 0xf5f7fb),
            Self::Text => (0xe3e5ea, 0x1d1f24),
            Self::Muted => (0x8b8f99, 0x676c78),
            Self::Subtle => (0x575b64, 0xa3a7b0),
            Self::Accent => (0x7c9cff, 0x3b6ef5),
            Self::Selection => (0x2d4270, 0xc7dbff),
            Self::Error => (0xf07575, 0xc9402f),
            Self::GitAdded => (0x73c991, 0x2f8a3f),
            Self::GitModified => (0x6fa8ff, 0x1a64d6),
            Self::GitUntracked => (0xe0776a, 0xc0392b),
            Self::GitDeleted => (0x868a93, 0x6c707e),
            Self::GitConflicted => (0xe8bd62, 0x9d6c00),
            Self::DiffAdded => (0x1f3a2b, 0xe3f6e5),
            Self::DiffRemoved => (0x42262a, 0xfde7e7),
            Self::DiffHunk => (0x1e2436, 0xeef3ff),
        }
    }
}

fn syntax_defaults(token: Token) -> (u32, u32) {
    match token {
        Token::Keyword => (0xc792ea, 0x8e3fc9),
        Token::Declaration => (0x82aaff, 0x2f5fd1),
        Token::Call => (0xe8c17a, 0x8a6400),
        Token::Name => (0xf0a3c0, 0xb8306a),
        Token::Type => (0x5fd4c4, 0x08806f),
        Token::Constant => (0xf78c6c, 0xc2531c),
        Token::String => (0xa5d68b, 0x3d8a2e),
        Token::Comment => (0x676b75, 0x8c8f96),
        Token::DocComment => (0x6f8f7b, 0x53785d),
        Token::Attribute => (0xc3bb74, 0x6f7520),
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hex(pub Rgba);

impl Serialize for Hex {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let color = hex(self.0);
        match (self.0.a * 255.).round() as u32 {
            0xff => serializer.collect_str(&format_args!("#{color:06x}")),
            alpha => serializer.collect_str(&format_args!("#{color:06x}{alpha:02x}")),
        }
    }
}

impl<'de> Deserialize<'de> for Hex {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Rgba::deserialize(deserializer).map(Self)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Palette {
    pub colors: BTreeMap<Color, Hex>,
    pub syntax: BTreeMap<Token, Hex>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Themes {
    pub dark: Palette,
    pub light: Palette,
}

impl Themes {
    pub fn defaults() -> Self {
        let palette = |dark: bool| Palette {
            colors: Color::ALL
                .into_iter()
                .map(|color| (color, Hex(choose(color.defaults(), dark))))
                .collect(),
            syntax: Token::ALL
                .into_iter()
                .map(|token| (token, Hex(choose(syntax_defaults(token), dark))))
                .collect(),
        };
        Self {
            dark: palette(true),
            light: palette(false),
        }
    }
}

pub fn mode() -> ThemeMode {
    MODE.get()
}

pub fn set_themes(themes: Themes) {
    THEMES.set(themes);
}

pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    MODE.set(mode);
    Theme::change(mode, window, cx);
    let theme = Theme::global_mut(cx);
    apply_component_theme(theme);
    let mut highlight = (*theme.highlight_theme).clone();
    let style = &mut highlight.style;
    style.syntax = syntax_colors();
    style.editor_background = Some(background().into());
    style.editor_foreground = Some(text().into());
    style.editor_active_line = Some(active_line().into());
    style.editor_line_number = Some(subtle().into());
    style.editor_active_line_number = Some(text().into());
    theme.highlight_theme = Arc::new(highlight);
}

pub fn init_fonts(cx: &App) {
    let installed = cx.text_system().all_font_names();
    if let Some(family) = PREFERRED_MONO
        .into_iter()
        .find(|family| installed.iter().any(|name| name == family))
    {
        MONO_FAMILY.set(family);
    }
}

fn apply_component_theme(theme: &mut Theme) {
    let dark = mode().is_dark();
    theme.radius = px(6.);
    theme.radius_lg = px(10.);
    theme.shadow = true;
    theme.mono_font_family = mono_family().into();

    let hsl = |color: Rgba| Hsla::from(color);
    let colors = &mut theme.colors;
    colors.background = hsl(background());
    colors.foreground = hsl(text());
    colors.border = hsl(border());
    colors.input = hsl(border_strong());
    colors.ring = hsl(accent());
    colors.caret = hsl(accent());
    colors.selection = hsl(selection());
    colors.link = hsl(accent());
    colors.link_hover = hsl(accent());
    colors.link_active = hsl(accent());
    colors.drag_border = hsl(accent());
    colors.drop_target = hsl(accent()).opacity(0.12);

    colors.muted = hsl(hover());
    colors.muted_foreground = hsl(muted());
    colors.accent = hsl(hover());
    colors.accent_foreground = hsl(text());
    colors.popover = hsl(elevated());
    colors.popover_foreground = hsl(text());
    colors.overlay = hsl(rgba(if dark { 0x0000_0099 } else { 0x0f11_1a40 }));

    colors.primary = hsl(pick(0x5b7cfa, 0x3b6ef5));
    colors.primary_hover = hsl(pick(0x6d8bff, 0x4d7cff));
    colors.primary_active = hsl(pick(0x4c6ef5, 0x2f5fe0));
    colors.primary_foreground = hsl(rgb(0xffffff));
    colors.secondary = hsl(pick(0x24262c, 0xeff1f4));
    colors.secondary_hover = hsl(pick(0x2b2e35, 0xe6e8ec));
    colors.secondary_active = hsl(pick(0x33363e, 0xdcdfe4));
    colors.secondary_foreground = hsl(text());
    colors.danger = hsl(error());
    colors.danger_hover = hsl(error()).opacity(0.9);
    colors.danger_active = hsl(error()).opacity(0.8);
    colors.danger_foreground = hsl(rgb(0xffffff));

    colors.list = hsl(panel());
    colors.list_even = hsl(panel());
    colors.list_head = hsl(panel());
    colors.list_hover = hsl(hover());
    colors.list_active = hsl(active_row());
    colors.list_active_border = hsl(accent());
    colors.sidebar = hsl(panel());
    colors.sidebar_border = hsl(border());
    colors.sidebar_foreground = hsl(text());
    colors.sidebar_accent = hsl(hover());
    colors.sidebar_accent_foreground = hsl(text());
    colors.title_bar = hsl(chrome());
    colors.title_bar_border = hsl(border());
    colors.window_border = hsl(border());
    colors.tab_bar = hsl(panel());
    colors.tab = hsl(panel());
    colors.tab_active = hsl(background());
    colors.tab_foreground = hsl(muted());
    colors.tab_active_foreground = hsl(text());

    colors.switch = hsl(pick(0x3a3d45, 0xd3d6dc));
    colors.switch_thumb = hsl(rgb(0xffffff));
    colors.scrollbar = Hsla::transparent_black();
    colors.scrollbar_thumb = hsl(rgba(if dark { 0xffff_ff1f } else { 0x0000_0024 }));
    colors.scrollbar_thumb_hover = hsl(rgba(if dark { 0xffff_ff38 } else { 0x0000_0040 }));
}

fn choose((dark, light): (u32, u32), is_dark: bool) -> Rgba {
    rgb(if is_dark { dark } else { light })
}

fn pick(dark: u32, light: u32) -> Rgba {
    choose((dark, light), mode().is_dark())
}

fn resolve(lookup: impl FnOnce(&Palette) -> Option<Hex>, defaults: (u32, u32)) -> Rgba {
    let dark = mode().is_dark();
    THEMES
        .with_borrow(|themes| lookup(if dark { &themes.dark } else { &themes.light }))
        .map_or_else(|| choose(defaults, dark), |hex| hex.0)
}

pub fn color(color: Color) -> Rgba {
    resolve(
        |palette| palette.colors.get(&color).copied(),
        color.defaults(),
    )
}

pub fn background() -> Rgba {
    color(Color::Background)
}

pub fn panel() -> Rgba {
    color(Color::Panel)
}

pub fn chrome() -> Rgba {
    color(Color::Chrome)
}

pub fn elevated() -> Rgba {
    color(Color::Elevated)
}

pub fn border() -> Rgba {
    color(Color::Border)
}

pub fn border_strong() -> Rgba {
    color(Color::BorderStrong)
}

pub fn hover() -> Rgba {
    color(Color::Hover)
}

pub fn active_row() -> Rgba {
    color(Color::ActiveRow)
}

pub fn active_line() -> Rgba {
    color(Color::ActiveLine)
}

pub fn text() -> Rgba {
    color(Color::Text)
}

pub fn muted() -> Rgba {
    color(Color::Muted)
}

pub fn subtle() -> Rgba {
    color(Color::Subtle)
}

pub fn accent() -> Rgba {
    color(Color::Accent)
}

pub fn accent_wash() -> Hsla {
    Hsla::from(accent()).opacity(if mode().is_dark() { 0.14 } else { 0.1 })
}

pub fn selection() -> Rgba {
    color(Color::Selection)
}

pub fn git_change(change: Change) -> Rgba {
    color(match change {
        Change::Added => Color::GitAdded,
        Change::Untracked => Color::GitUntracked,
        Change::Deleted => Color::GitDeleted,
        Change::Conflicted => Color::GitConflicted,
        Change::Modified | Change::Renamed | Change::Copied | Change::TypeChanged => {
            Color::GitModified
        }
    })
}

pub fn file_icon(kind: FileKind) -> Rgba {
    match kind {
        FileKind::GitIgnore => pick(0xe8735f, 0xc14a34),
        FileKind::JavaScript => pick(0xe3c25f, 0x8a6d00),
        FileKind::Json => pick(0x8fca8a, 0x2f7d3a),
        FileKind::Lock => pick(0x9096a0, 0x6c717b),
        FileKind::Markdown => pick(0x6fc9c4, 0x14807a),
        FileKind::Rust => pick(0xdca07a, 0xa9591f),
        FileKind::Toml => pick(0xb79ae0, 0x6a3fb5),
        FileKind::TypeScript => pick(0x6aa9f0, 0x1f6fd0),
        FileKind::Plain => muted(),
    }
}

pub fn diff_added() -> Rgba {
    color(Color::DiffAdded)
}

pub fn diff_removed() -> Rgba {
    color(Color::DiffRemoved)
}

pub fn diff_hunk() -> Rgba {
    color(Color::DiffHunk)
}

pub fn error() -> Rgba {
    color(Color::Error)
}

pub fn syntax(token: Token) -> Rgba {
    resolve(
        |palette| palette.syntax.get(&token).copied(),
        syntax_defaults(token),
    )
}

pub fn syntax_colors() -> SyntaxColors {
    let styles = Token::ALL
        .into_iter()
        .map(|token| {
            let key = match token {
                Token::DocComment => "comment_doc",
                _ => token.capture(),
            };
            let style = serde_json::json!({ "color": Hex(syntax(token)) });
            (key.to_string(), style)
        })
        .collect();
    serde_json::from_value(serde_json::Value::Object(styles))
        .expect("syntax colors are valid theme styles")
}

pub fn hex(color: Rgba) -> u32 {
    let channel = |value: f32| (value * 255.).round() as u32;
    channel(color.r) << 16 | channel(color.g) << 8 | channel(color.b)
}

pub fn mono_family() -> &'static str {
    MONO_FAMILY.get()
}

pub fn monospace_font() -> Font {
    Font {
        fallbacks: Some(FontFallbacks::from_fonts(vec![FALLBACK_MONO.into()])),
        ..font(mono_family())
    }
}

pub fn terminal_palette() -> terminal::Palette {
    let defaults = terminal::Palette::default();
    terminal::Palette {
        foreground: hex(text()),
        background: hex(background()),
        cursor: hex(accent()),
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
