//! Loading, applying and editing settings files.

use super::*;

impl IdeShell {
    /// Re-read the settings layers (so edits made outside the IDE apply) and return the
    /// effective settings. Invalid layers are reported; their last valid contents apply.
    pub(super) fn load_settings(&mut self, cx: &mut Context<Self>) -> Option<Settings> {
        let root = self.settings_root();
        let problems = settings::reload(root.as_deref(), None, cx);
        if let Some(summary) = settings::summary(&problems) {
            self.status = summary;
            cx.notify();
        }
        Some(settings::current(cx))
    }

    pub(super) fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) -> String {
        let root = self.settings_root();
        let problems = settings::reload(root.as_deref(), Some(window), cx);
        self.show_sidebar = settings::current(cx).show_sidebar;
        self.sync_servers(cx);
        self.refresh_preferences(None, cx);
        cx.notify();
        settings::summary(&problems).unwrap_or_else(|| "Settings applied".into())
    }

    pub(super) fn reload_settings(
        &mut self,
        _: &ReloadSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.status = self.apply_settings(window, cx);
    }

    pub(super) fn edit_settings(
        &mut self,
        _: &EditSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Some(path) = settings::path(cx) else {
            self.status = "No settings folder is available on this system".into();
            cx.notify();
            return;
        };
        if let Err(error) = settings::create_if_missing(&path) {
            self.status = format!("Could not create {}: {error}", path.display());
            cx.notify();
            return;
        }
        self.open(Some(path), window, cx);
    }

    pub(super) fn open_settings(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let shell = cx.entity().downgrade();
        let vim_enabled = Rc::new(Cell::new(self.vim.is_some()));
        let settings_path = settings::path(cx);
        window.open_dialog(cx, move |dialog, _, _| {
            let (shell, vim_enabled) = (shell.clone(), vim_enabled.clone());
            let settings_file = {
                let (edit_shell, reload_shell) = (shell.clone(), shell.clone());
                let location = settings_path
                    .as_ref()
                    .map_or("No settings folder is available".into(), |path| {
                        path.display().to_string()
                    });
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .child(settings::FILE_NAME)
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::muted())
                                    .child("Hotkeys, colors, tasks and language servers"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme::subtle())
                                    .truncate()
                                    .child(location),
                            ),
                    )
                    .child(
                        Button::new("reload-settings")
                            .ghost()
                            .small()
                            .label("Reload")
                            .disabled(settings_path.is_none())
                            .on_click(move |_, window, cx| {
                                let _ = reload_shell.update(cx, |shell, cx| {
                                    shell.reload_settings(&ReloadSettings, window, cx)
                                });
                            }),
                    )
                    .child(
                        div().debug_selector(|| "edit-settings".into()).child(
                            Button::new("edit-settings")
                                .small()
                                .label("Edit")
                                .disabled(settings_path.is_none())
                                .on_click(move |_, window, cx| {
                                    window.close_dialog(cx);
                                    let _ = edit_shell.update(cx, |shell, cx| {
                                        shell.edit_settings(&EditSettings, window, cx)
                                    });
                                }),
                        ),
                    )
            };
            let section = |title: &'static str, setting: Div| {
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(ui::caption(title))
                    .child(
                        div()
                            .px_3()
                            .py_2p5()
                            .rounded_lg()
                            .border_1()
                            .border_color(theme::border())
                            .bg(theme::panel())
                            .child(setting),
                    )
            };
            dialog.title("Settings").w(px(440.)).child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .pb_1()
                    .child(section(
                        "APPEARANCE",
                        div().debug_selector(|| "light-mode".into()).child(
                            Switch::new("light-mode")
                                .label("Light mode")
                                .checked(!theme::mode().is_dark())
                                .on_click(|light, window, cx| {
                                    let mode = if *light {
                                        ThemeMode::Light
                                    } else {
                                        ThemeMode::Dark
                                    };
                                    theme::set_mode(mode, Some(window), cx);
                                }),
                        ),
                    ))
                    .child(section(
                        "EDITOR",
                        div().debug_selector(|| "vim-mode".into()).child(
                            Switch::new("vim-mode")
                                .label("Vim keys in the editor")
                                .checked(vim_enabled.get())
                                .on_click(move |enabled, _, cx| {
                                    vim_enabled.set(*enabled);
                                    let _ = shell
                                        .update(cx, |shell, cx| shell.set_vim_mode(*enabled, cx));
                                }),
                        ),
                    ))
                    .child(section("CUSTOMIZATION", settings_file)),
            )
        });
    }
}
