#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod commands;
mod shell;
mod terminal_view;
mod theme;

use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, ThemeMode, TitleBar};
use gpui_component_assets::Assets;
use shell::IdeShell;

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        gpui_component::init(cx);
        theme::set_mode(ThemeMode::Dark, None, cx);
        commands::init(cx);
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1200.0), px(800.0)), cx);
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(720.0), px(480.0))),
                titlebar: Some(TitleBar::title_bar_options()),
                ..Default::default()
            },
            |window, cx| {
                window.set_window_title("Hephaestus");
                let shell = gpui::AppContext::new(cx, |cx| IdeShell::new(window, cx));
                let weak_shell = shell.downgrade();
                window.on_window_should_close(cx, move |window, cx| {
                    weak_shell
                        .update(cx, |shell, cx| shell.can_close(window, cx))
                        .unwrap_or(true)
                });
                gpui::AppContext::new(cx, |cx| Root::new(shell, window, cx))
            },
        )
        .expect("failed to open the Hephaestus window");
        cx.activate(true);
    });
}
