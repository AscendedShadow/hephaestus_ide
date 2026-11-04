//! The window layout and the actions it routes to the shell.

use super::*;

impl Render for IdeShell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.schedule_fs_batch(window, cx);
        let (name, dirty) = self.active_tab(cx);
        let name = format!("{name}{}", if dirty { " •" } else { "" });
        window.set_window_title(&match self.workspace.root() {
            Some(_) => format!("{name} — {} — Hephaestus", self.workspace.display_name()),
            None => format!("{name} — Hephaestus"),
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .text_color(theme::text())
            .text_size(px(13.))
            .on_action(cx.listener(Self::new_file))
            .on_action(cx.listener(Self::open_file))
            .on_action(cx.listener(Self::open_folder))
            .on_action(cx.listener(Self::clone_repository))
            .on_action(cx.listener(Self::save_file))
            .on_action(cx.listener(Self::save_file_as))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::previous_tab))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::edit_settings))
            .on_action(cx.listener(Self::reload_settings))
            .on_action(cx.listener(Self::show_folder_panel))
            .on_action(cx.listener(Self::toggle_terminal))
            .on_action(cx.listener(Self::show_git_panel))
            .on_action(cx.listener(Self::show_debug_panel))
            .on_action(cx.listener(Self::toggle_vim_mode))
            .on_action(cx.listener(Self::toggle_fold))
            .on_action(cx.listener(Self::quick_open))
            .on_action(cx.listener(Self::command_palette))
            .on_action(cx.listener(Self::search_project))
            .on_action(cx.listener(Self::show_diagnostics))
            .on_action(cx.listener(Self::go_to_definition_action))
            .on_action(cx.listener(Self::hover_info))
            .on_action(cx.listener(Self::complete_code))
            .on_action(cx.listener(Self::toggle_breakpoint))
            .on_action(cx.listener(Self::debug_continue))
            .on_action(cx.listener(Self::debug_step_over))
            .on_action(cx.listener(Self::debug_step_into))
            .on_action(cx.listener(Self::debug_step_out))
            .on_action(cx.listener(Self::debug_interrupt))
            .on_action(cx.listener(Self::debug_stop))
            .on_action(cx.listener(Self::debug_restart))
            .on_action(cx.listener(Self::go_back))
            .on_action(cx.listener(Self::go_forward))
            .on_action(cx.listener(Self::go_to_line))
            .on_action(cx.listener(Self::toggle_line_comment))
            .on_action(cx.listener(Self::toggle_block_comment))
            .on_action(cx.listener(Self::move_lines_up))
            .on_action(cx.listener(Self::move_lines_down))
            .on_action(cx.listener(Self::duplicate_lines_up))
            .on_action(cx.listener(Self::duplicate_lines_down))
            .on_action(cx.listener(Self::delete_lines))
            .on_action(cx.listener(Self::insert_line_above))
            .on_action(cx.listener(Self::insert_line_below))
            .on_action(cx.listener(Self::open_workspace_settings))
            .on_action(cx.listener(Self::open_user_settings))
            .on_action(cx.listener(Self::show_effective_settings))
            .on_action(cx.listener(Self::save_all_action))
            .on_action(cx.listener(Self::undo_workspace_replace))
            .on_action(cx.listener(Self::undo_workspace_edit))
            .on_action(cx.listener(Self::rename_symbol))
            .on_action(cx.listener(Self::find_references))
            .on_action(cx.listener(Self::find_references_without_declaration))
            .on_action(cx.listener(Self::go_to_implementation))
            .on_action(cx.listener(Self::go_to_type_definition))
            .on_action(cx.listener(Self::go_to_symbol_in_file))
            .on_action(cx.listener(Self::go_to_symbol_in_workspace))
            .on_action(cx.listener(Self::format_document))
            .on_action(cx.listener(Self::format_selection))
            .on_action(cx.listener(Self::quick_fix))
            .on_action(cx.listener(Self::refactor))
            .on_action(cx.listener(Self::source_action))
            .on_action(cx.listener(Self::organize_imports))
            .on_action(cx.listener(Self::show_outline))
            .on_action(cx.listener(Self::trigger_parameter_hints))
            .on_action(cx.listener(Self::next_signature))
            .on_action(cx.listener(Self::previous_signature))
            .on_action(cx.listener(Self::next_tab_stop))
            .on_action(cx.listener(Self::previous_tab_stop))
            .on_action(cx.listener(Self::explorer_new_file))
            .on_action(cx.listener(Self::explorer_new_folder))
            .on_action(cx.listener(Self::rename_item))
            .on_action(cx.listener(Self::move_item))
            .on_action(cx.listener(Self::delete_item))
            .on_action(cx.listener(Self::new_terminal))
            .on_action(cx.listener(Self::next_terminal))
            .on_action(cx.listener(Self::close_terminal))
            .on_action(cx.listener(Self::rename_terminal))
            .on_action(cx.listener(Self::select_terminal_profile))
            .on_action(cx.listener(Self::refresh_explorer))
            .on_action(cx.listener(Self::reopen_with_encoding))
            .on_action(cx.listener(Self::save_with_encoding))
            .on_action(cx.listener(Self::change_line_ending))
            .on_action(cx.listener(Self::select_debug_profile))
            .on_action(cx.listener(Self::edit_breakpoint))
            .on_action(cx.listener(Self::toggle_breakpoint_enabled))
            .on_action(cx.listener(Self::remove_all_breakpoints))
            .on_action(cx.listener(Self::add_watch))
            .on_action(cx.listener(Self::run_task_picker))
            .on_action(cx.listener(Self::run_build_task))
            .on_action(cx.listener(Self::cancel_task))
            .on_action(cx.listener(Self::rerun_last_task))
            .on_action(cx.listener(Self::show_problems))
            .on_action(cx.listener(Self::next_problem))
            .on_action(cx.listener(Self::previous_problem))
            .on_action(cx.listener(Self::switch_branch))
            .on_action(cx.listener(Self::show_history))
            .on_action(cx.listener(Self::show_file_history))
            .on_action(cx.listener(Self::blame_line))
            .on_action(cx.listener(Self::blame_file))
            .on_action(cx.listener(Self::stage_hunk))
            .on_action(cx.listener(Self::unstage_hunk))
            .on_action(cx.listener(Self::show_testing))
            .on_action(cx.listener(Self::refresh_tests))
            .on_action(cx.listener(Self::run_all_tests))
            .on_action(cx.listener(Self::rerun_failed_tests))
            .on_action(cx.listener(Self::cancel_tests))
            .on_action(cx.listener(Self::add_cursor_above))
            .on_action(cx.listener(Self::add_cursor_below))
            .on_action(cx.listener(Self::select_next_occurrence))
            .on_action(cx.listener(Self::select_all_occurrences))
            .on_action(cx.listener(Self::split_editor_right))
            .on_action(cx.listener(Self::split_editor_down))
            .on_action(cx.listener(Self::close_editor_group))
            .on_action(cx.listener(Self::focus_next_group))
            .on_action(cx.listener(Self::focus_previous_group))
            .on_action(cx.listener(Self::move_tab_to_next_group))
            .on_action(cx.listener(Self::add_folder_to_workspace))
            .on_action(cx.listener(Self::remove_folder_from_workspace))
            .on_action(cx.listener(Self::select_active_root))
            .on_action(cx.listener(Self::toggle_root_search))
            .on_action(cx.listener(Self::save_workspace_as))
            .on_action(cx.listener(Self::change_language_mode))
            .on_action(cx.listener(Self::find_in_file))
            .on_action(cx.listener(Self::run_command))
            .on_action(cx.listener(Self::stop_command))
            .on_action(cx.listener(Self::start_debugger))
            .on_action(cx.listener(Self::reload_file))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::stage_all)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::pull)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::push)))
            .on_action(cx.listener(Self::forward_to_git(GitPanel::fetch)))
            .child(self.render_title_bar(cx))
            .when(self.buffer().external_change, |view| {
                view.child(
                    div()
                        .px_3()
                        .py_1()
                        .flex()
                        .items_center()
                        .gap_3()
                        .bg(theme::panel())
                        .child(if self.buffer().deleted_on_disk {
                            "File was deleted on disk. Save As to keep this buffer."
                        } else {
                            "File changed on disk. Reload discards your edits."
                        })
                        .child(
                            Button::new("reload-external")
                                .small()
                                .label("Reload")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.reload_file(&ReloadFile, window, cx)
                                })),
                        )
                        .child(
                            Button::new("save-external-as")
                                .small()
                                .label("Save As…")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.save_file_as(&SaveFileAs, window, cx)
                                })),
                        ),
                )
            })
            .when(self.search_mode.is_some(), |view| {
                view.child(self.render_search(cx))
            })
            .when(self.pending.is_some(), |view| {
                view.child(self.render_save_prompt(cx))
            })
            .child(
                div().flex_1().min_h_0().flex().child(
                    div().flex_1().min_w_0().h_full().child(
                        h_resizable("sidebar-split")
                            .with_state(&self.sidebar_split)
                            .when(self.show_sidebar, |split| {
                                split.child(
                                    resizable_panel()
                                        .size(px(240.))
                                        .size_range(px(160.)..px(480.))
                                        .child(self.render_sidebar(cx)),
                                )
                            })
                            .child(
                                v_resizable("tool-panel-split")
                                    .with_state(&self.tool_panel_split)
                                    .child(resizable_panel().child({
                                        let editor =
                                            self.render_editor(window, cx).into_any_element();
                                        self.render_groups(editor, cx)
                                    }))
                                    .child(
                                        resizable_panel()
                                            .size(px(200.))
                                            .size_range(px(72.)..Pixels::MAX)
                                            .child(self.render_tool_panel(cx)),
                                    ),
                            ),
                    ),
                ),
            )
            .child(self.render_status_bar(cx))
            .children(Root::render_dialog_layer(window, cx))
    }
}
