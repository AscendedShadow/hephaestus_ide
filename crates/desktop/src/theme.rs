//! Shared colors for the dark and light themes.

use std::cell::Cell;

use gpui::{App, Font, Rgba, Window, font, rgb};
use gpui_component::{Theme, ThemeMode};

thread_local! {
    // UI rendering happens on a single thread, so the active mode lives beside it
    // instead of being threaded through every color lookup.
    static MODE: Cell<ThemeMode> = const { Cell::new(ThemeMode::Dark) };
}

pub fn mode() -> ThemeMode {
    MODE.get()
}

/// Switch the shell palette and the gpui-component theme together.
pub fn set_mode(mode: ThemeMode, window: Option<&mut Window>, cx: &mut App) {
    MODE.set(mode);
    Theme::change(mode, window, cx);
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

pub fn monospace_font() -> Font {
    font(if cfg!(target_os = "windows") {
        "Consolas"
    } else {
        "monospace"
    })
}

/// Terminal colors on the editor background, with ANSI colors readable in each mode.
pub fn terminal_palette() -> terminal::Palette {
    let hex = |color: Rgba| {
        let channel = |value: f32| (value * 255.).round() as u32;
        channel(color.r) << 16 | channel(color.g) << 8 | channel(color.b)
    };
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
