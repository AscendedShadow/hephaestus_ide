#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod assets;
mod brace_guide;
mod commands;
mod diff_view;
mod folding;
mod git_panel;
mod settings;
mod shell;
mod syntax;
mod terminal_view;
mod theme;
mod ui;
mod vim;

use assets::Assets;
use gpui::{App, Application, Bounds, WindowBounds, WindowOptions, px, size};
use gpui_component::{Root, TitleBar};
use shell::IdeShell;

fn main() {
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        gpui_component::init(cx);
        syntax::init();
        theme::init_fonts(cx);
        let settings_status = settings::summary(&settings::init(cx));
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
                let shell = gpui::AppContext::new(cx, |cx| {
                    let mut shell = IdeShell::new(window, cx);
                    if let Some(status) = settings_status {
                        shell.set_status(status);
                    }
                    shell
                });
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
