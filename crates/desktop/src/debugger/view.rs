//! Rendering the Debug panel: toolbar, call stack, breakpoints, variables, console
//! and memory views.

use super::*;

pub(super) fn section_header(title: &'static str, count: Option<usize>) -> Div {
    div()
        .h(px(26.))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1p5()
        .px_3()
        .child(ui::caption(title))
        .when_some(count, |header, count| header.child(ui::count_badge(count)))
}

pub(super) fn row(id: impl Into<gpui::ElementId>, selected: bool) -> Stateful<Div> {
    div()
        .id(id)
        .group(ROW_GROUP)
        .h(px(ROW_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_center()
        .gap_1p5()
        .px_2()
        .mx_1()
        .rounded_md()
        .text_xs()
        .cursor_pointer()
        .when(selected, |row| row.bg(theme::accent_wash()))
        .when(!selected, |row| row.hover(|style| style.bg(theme::hover())))
}

fn breakpoint_dot(rejected: bool) -> Div {
    let color = theme::breakpoint();
    div()
        .size(px(9.))
        .flex_shrink_0()
        .rounded_full()
        .when(rejected, |dot| dot.border_1().border_color(color))
        .when(!rejected, |dot| dot.bg(color))
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

impl Debugger {
    fn tool_button(
        &self,
        id: &'static str,
        icon: AppIcon,
        tooltip: &'static str,
        action: &dyn Action,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let boxed = action.boxed_clone();
        Button::new(id)
            .ghost()
            .small()
            .icon(Icon::new(icon).size(px(15.)))
            .tooltip_with_action(tooltip, action, None)
            .disabled(!enabled)
            .on_click(cx.listener(move |_, _, window, cx| {
                window.dispatch_action(boxed.boxed_clone(), cx);
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let active = self.is_active();
        let paused = self.is_paused();
        let running = matches!(self.state, State::Running | State::Starting);
        let continue_button = if active {
            self.tool_button(
                "debug-continue",
                AppIcon::Play,
                "Continue",
                &DebugContinue,
                paused,
                cx,
            )
        } else {
            self.tool_button(
                "debug-start",
                AppIcon::Play,
                "Start Debugging",
                &StartDebugger,
                true,
                cx,
            )
        };
        div()
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(theme::border())
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_0p5()
                    .p_0p5()
                    .rounded_md()
                    .bg(theme::panel())
                    .border_1()
                    .border_color(theme::border())
                    .child(continue_button.when(!active || paused, |button| {
                        button.text_color(theme::git_change(ide_core::git::Change::Added))
                    }))
                    .child(self.tool_button(
                        "debug-pause",
                        AppIcon::Pause,
                        "Pause",
                        &DebugInterrupt,
                        active && running,
                        cx,
                    ))
                    .child(div().w(px(1.)).h(px(16.)).mx_0p5().bg(theme::border()))
                    .child(self.tool_button(
                        "debug-next",
                        AppIcon::StepOver,
                        "Step Over",
                        &DebugStepOver,
                        paused,
                        cx,
                    ))
                    .child(self.tool_button(
                        "debug-step",
                        AppIcon::StepInto,
                        "Step Into",
                        &DebugStepInto,
                        paused,
                        cx,
                    ))
                    .child(self.tool_button(
                        "debug-step-out",
                        AppIcon::StepOut,
                        "Step Out",
                        &DebugStepOut,
                        paused,
                        cx,
                    ))
                    .child(div().w(px(1.)).h(px(16.)).mx_0p5().bg(theme::border()))
                    .child(self.tool_button(
                        "debug-restart",
                        AppIcon::Restart,
                        "Restart",
                        &DebugRestart,
                        self.last_launch.is_some(),
                        cx,
                    ))
                    .child(
                        self.tool_button(
                            "debug-stop",
                            AppIcon::Stop,
                            "Stop",
                            &DebugStop,
                            active,
                            cx,
                        )
                        .when(active, |button| button.text_color(theme::breakpoint())),
                    ),
            )
            .child(
                div()
                    .h(px(20.))
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .px_2()
                    .rounded_full()
                    .bg(theme::hover())
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .child(div().size(px(7.)).rounded_full().bg(self.state.color()))
                    .child(self.state.label()),
            )
            .when(!self.title.is_empty(), |bar| {
                bar.child(
                    div()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme::muted())
                        .child(self.title.clone()),
                )
            })
    }

    fn render_call_stack(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .child(section_header("CALL STACK", None))
            .when(self.frames.is_empty(), |list| {
                list.child(
                    div()
                        .px_3()
                        .pb_2()
                        .text_xs()
                        .text_color(theme::subtle())
                        .child(if self.is_paused() {
                            "Loading…"
                        } else {
                            "Not paused"
                        }),
                )
            })
            .children(self.frames.iter().enumerate().map(|(ix, frame)| {
                let selected = ix == self.frame;
                let location = frame
                    .path()
                    .map(|path| format!("{}:{}", file_name(path), frame.line))
                    .or_else(|| frame.instruction_pointer_reference.clone())
                    .unwrap_or_default();
                let has_source = frame.path().is_some_and(|path| path.is_file());
                row(("debug-frame", ix), selected)
                    .child(
                        div()
                            .w(px(12.))
                            .flex_shrink_0()
                            .text_color(theme::execution_line())
                            .when(ix == 0, |slot| slot.child("➜")),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .font_family(theme::mono_family())
                            .text_color(if has_source {
                                theme::text()
                            } else {
                                theme::muted()
                            })
                            .child(frame.name.clone()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(theme::subtle())
                            .child(location),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_frame(ix, cx)))
            }))
    }

    fn render_breakpoints(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut entries: Vec<(&PathBuf, usize)> = self
            .breakpoints
            .iter()
            .flat_map(|(path, lines)| lines.iter().map(move |line| (path, *line)))
            .collect();
        entries.sort();
        div()
            .flex()
            .flex_col()
            .pb_2()
            .child(
                section_header("BREAKPOINTS", Some(entries.len()))
                    .child(div().flex_1())
                    .when(!entries.is_empty(), |header| {
                        header.child(
                            Button::new("debug-clear-breakpoints")
                                .ghost()
                                .xsmall()
                                .icon(IconName::Delete)
                                .tooltip("Remove all breakpoints")
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.remove_all_breakpoints(cx)),
                                ),
                        )
                    }),
            )
            .when(entries.is_empty(), |list| {
                list.child(
                    div()
                        .px_3()
                        .text_xs()
                        .text_color(theme::subtle())
                        .child("Click left of a line number to add one"),
                )
            })
            .children(entries.into_iter().enumerate().map(|(ix, (path, line))| {
                let rejected = self.is_rejected(path, line);
                let reveal = path.clone();
                let remove = path.clone();
                let toggle = path.clone();
                let options = self.options_at(path, line);
                let summary = options.summary();
                row(("debug-breakpoint", ix), false)
                    .child(
                        gpui_component::checkbox::Checkbox::new(("debug-breakpoint-enabled", ix))
                            .checked(options.enabled)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.toggle_breakpoint_enabled(toggle.clone(), line, cx);
                            })),
                    )
                    .child(breakpoint_dot(rejected || !options.enabled))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(if options.enabled {
                                theme::text()
                            } else {
                                theme::subtle()
                            })
                            .child(if summary.is_empty() {
                                file_name(path)
                            } else {
                                format!("{} — {summary}", file_name(path))
                            }),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .font_family(theme::mono_family())
                            .text_color(theme::subtle())
                            .child(line.to_string()),
                    )
                    .child(
                        div()
                            .invisible()
                            .group_hover(ROW_GROUP, |style| style.visible())
                            .child(
                                Button::new(("debug-remove-breakpoint", ix))
                                    .ghost()
                                    .xsmall()
                                    .icon(IconName::Close)
                                    .tooltip("Remove breakpoint")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        cx.stop_propagation();
                                        this.toggle_breakpoint(remove.clone(), line, cx);
                                    })),
                            ),
                    )
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(DebuggerEvent::Reveal {
                            path: reveal.clone(),
                            line,
                        })
                    }))
            }))
    }

    pub(super) fn render_variable(
        &self,
        variable: &Variable,
        depth: usize,
        key: &mut usize,
        rows: &mut Vec<AnyElement>,
        cx: &mut Context<Self>,
    ) {
        let reference = variable.variables_reference;
        let expandable = reference > 0;
        let expanded = expandable && self.expanded.contains(&reference);
        let memory = pointer_reference(variable);
        let label = variable
            .evaluate_name
            .clone()
            .unwrap_or_else(|| variable.name.clone());
        *key += 1;
        let ix = *key;
        rows.push(
            row(("debug-variable", ix), false)
                .pl(px(8. + depth as f32 * 14.))
                .font_family(theme::mono_family())
                .child(
                    div()
                        .w(px(12.))
                        .flex_shrink_0()
                        .text_color(theme::subtle())
                        .when(expandable, |slot| {
                            slot.child(
                                Icon::new(if expanded {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size(px(12.)),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(theme::syntax(crate::syntax::Token::Name))
                        .child(variable.name.clone()),
                )
                .child(div().flex_shrink_0().text_color(theme::subtle()).child("="))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(theme::text())
                        .child(variable.value.clone()),
                )
                .when_some(
                    variable.type_name.clone().filter(|name| !name.is_empty()),
                    |row, type_name| {
                        row.child(
                            div()
                                .flex_shrink_0()
                                .max_w(px(160.))
                                .truncate()
                                .text_color(theme::subtle())
                                .child(type_name),
                        )
                    },
                )
                .child(
                    div()
                        .w(px(22.))
                        .flex_shrink_0()
                        .when_some(memory, |slot, memory| {
                            slot.invisible()
                                .group_hover(ROW_GROUP, |style| style.visible())
                                .child(
                                    Button::new(("debug-variable-memory", ix))
                                        .ghost()
                                        .xsmall()
                                        .icon(Icon::new(AppIcon::Memory))
                                        .tooltip("View memory")
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            cx.stop_propagation();
                                            this.show_memory(label.clone(), memory.clone(), cx);
                                        })),
                                )
                        }),
                )
                .when(expandable, |row| {
                    row.on_click(
                        cx.listener(move |this, _, _, cx| this.toggle_variable(reference, cx)),
                    )
                })
                .into_any_element(),
        );
        if expanded {
            match self.children.get(&reference) {
                Some(children) => {
                    for child in children {
                        self.render_variable(child, depth + 1, key, rows, cx);
                    }
                }
                None => {
                    *key += 1;
                    rows.push(
                        div()
                            .pl(px(30. + depth as f32 * 14.))
                            .h(px(ROW_HEIGHT))
                            .text_xs()
                            .text_color(theme::subtle())
                            .child("Loading…")
                            .into_any_element(),
                    );
                }
            }
        }
    }

    fn render_variables(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut rows = Vec::new();
        let mut key = 0;
        for (ix, (scope, variables)) in self.scopes.iter().enumerate() {
            let collapsed = variables.is_none() || self.collapsed.contains(&scope.name);
            rows.push(
                row(("debug-scope", ix), false)
                    .mt_1()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(12.))
                        .text_color(theme::subtle()),
                    )
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme::muted())
                            .child(scope.name.clone()),
                    )
                    .when_some(variables.as_ref(), |row, variables| {
                        row.child(ui::count_badge(variables.len()))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle_scope(ix, cx)))
                    .into_any_element(),
            );
            if let Some(variables) = variables.as_ref().filter(|_| !collapsed) {
                if variables.is_empty() {
                    rows.push(
                        div()
                            .pl(px(30.))
                            .h(px(ROW_HEIGHT))
                            .text_xs()
                            .text_color(theme::subtle())
                            .child("No variables")
                            .into_any_element(),
                    );
                }
                for variable in variables {
                    self.render_variable(variable, 1, &mut key, &mut rows, cx);
                }
            }
        }
        let body =
            if !self.is_active() {
                ui::empty_state(
                    Icon::new(AppIcon::Bug).size(px(28.)),
                    "Press F5 to start debugging",
                )
                .child(div().text_xs().text_color(theme::subtle()).child(
                    "Set \"debugger\" in settings.json — click the gutter to add breakpoints",
                ))
                .into_any_element()
            } else if !self.is_paused() {
                ui::empty_state(
                    Icon::new(AppIcon::Play).size(px(24.)),
                    "Variables appear when the program pauses",
                )
                .into_any_element()
            } else if rows.is_empty() {
                div()
                    .p_3()
                    .text_xs()
                    .text_color(theme::subtle())
                    .child("Loading variables…")
                    .into_any_element()
            } else {
                div()
                    .flex()
                    .flex_col()
                    .pb_2()
                    .children(rows)
                    .into_any_element()
            };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(section_header("VARIABLES", None))
            .child(
                div()
                    .id("debug-variables")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }

    fn render_detail_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab =
            |id: &'static str, label: &'static str, detail: Detail, cx: &mut Context<Self>| {
                let active = self.detail == detail;
                div()
                    .id(id)
                    .h(px(22.))
                    .flex()
                    .items_center()
                    .px_2()
                    .rounded_md()
                    .cursor_pointer()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .map(|tab| {
                        if active {
                            tab.bg(theme::hover()).text_color(theme::text())
                        } else {
                            tab.text_color(theme::muted())
                                .hover(|style| style.text_color(theme::text()))
                        }
                    })
                    .child(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.detail = detail;
                        cx.notify();
                    }))
            };
        div()
            .h(px(30.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .child(tab("debug-tab-console", "Console", Detail::Console, cx))
            .child(tab("debug-tab-memory", "Memory", Detail::Memory, cx))
    }

    fn render_console(&self) -> impl IntoElement {
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(
                div()
                    .id("debug-console")
                    .flex_1()
                    .min_h_0()
                    .px_3()
                    .track_scroll(&self.console_scroll)
                    .overflow_y_scroll()
                    .font_family(theme::mono_family())
                    .text_xs()
                    .children(self.console.iter().map(|line| {
                        let (prefix, color): (&str, Hsla) = match line.kind {
                            LineKind::Output => ("", theme::text().into()),
                            LineKind::Error => ("", theme::error().into()),
                            LineKind::Input => ("› ", theme::accent().into()),
                            LineKind::Result => {
                                ("  ", theme::syntax(crate::syntax::Token::String).into())
                            }
                            LineKind::Info => ("", theme::subtle().into()),
                        };
                        div()
                            .whitespace_nowrap()
                            .text_color(color)
                            .child(format!("{prefix}{}", line.text))
                    })),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .border_t_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .text_color(theme::accent())
                            .font_family(theme::mono_family())
                            .text_xs()
                            .child("›"),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(Input::new(&self.console_input).small().appearance(false)),
                    ),
            )
    }

    fn render_memory(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let paging = self
            .memory
            .as_ref()
            .is_some_and(|memory| !memory.reference.is_empty() && self.is_active());
        let header = div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .pb_1p5()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.memory_input).small()),
            )
            .child(
                Button::new("debug-memory-go")
                    .small()
                    .label("Go")
                    .on_click(cx.listener(|this, _, _, cx| {
                        let expression = this.memory_input.read(cx).text().to_string();
                        this.inspect_memory(expression.trim().to_string(), cx);
                    })),
            )
            .child(
                Button::new("debug-memory-previous")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronUp)
                    .tooltip("Previous 256 bytes")
                    .disabled(!paging)
                    .on_click(cx.listener(|this, _, _, cx| this.page_memory(-1, cx))),
            )
            .child(
                Button::new("debug-memory-next")
                    .ghost()
                    .small()
                    .icon(IconName::ChevronDown)
                    .tooltip("Next 256 bytes")
                    .disabled(!paging)
                    .on_click(cx.listener(|this, _, _, cx| this.page_memory(1, cx))),
            );
        let body = match &self.memory {
            None => ui::empty_state(
                Icon::new(AppIcon::Memory).size(px(24.)),
                "Inspect memory at an address or expression",
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme::subtle())
                    .child("Or hover a variable and click its memory icon"),
            )
            .into_any_element(),
            Some(memory) => {
                let summary = match memory.start {
                    Some(start) => format!("{} @ 0x{:x}", memory.label, start),
                    None => memory.label.clone(),
                };
                div()
                    .flex()
                    .flex_col()
                    .px_3()
                    .pb_2()
                    .font_family(theme::mono_family())
                    .text_xs()
                    .child(
                        div()
                            .pb_1()
                            .text_color(theme::muted())
                            .truncate()
                            .child(summary),
                    )
                    .when_some(memory.error.clone(), |body, error| {
                        body.child(div().text_color(theme::error()).child(error))
                    })
                    .when(memory.loading, |body| {
                        body.child(div().text_color(theme::subtle()).child("Reading…"))
                    })
                    .when(memory.unreadable && !memory.loading, |body| {
                        body.child(
                            div()
                                .text_color(theme::subtle())
                                .child("Some bytes in this range are unreadable"),
                        )
                    })
                    .children(
                        hex_rows(memory.start, memory.offset, &memory.bytes)
                            .into_iter()
                            .enumerate()
                            .map(|(ix, (address, hex, ascii))| {
                                div()
                                    .flex()
                                    .gap_3()
                                    .px_1()
                                    .rounded_sm()
                                    .whitespace_nowrap()
                                    .when(ix % 2 == 1, |row| row.bg(theme::panel()))
                                    .child(div().text_color(theme::subtle()).child(address))
                                    .child(div().text_color(theme::text()).child(hex))
                                    .child(
                                        div()
                                            .text_color(theme::syntax(crate::syntax::Token::String))
                                            .child(ascii),
                                    )
                            }),
                    )
                    .into_any_element()
            }
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(header)
            .child(
                div()
                    .id("debug-memory")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scrollbar()
                    .child(body),
            )
    }
}

impl Render for Debugger {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let detail = match self.detail {
            Detail::Console => self.render_console().into_any_element(),
            Detail::Memory => self.render_memory(cx).into_any_element(),
        };
        div()
            .debug_selector(|| "debug-panel".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .child(self.render_toolbar(cx))
            .child(
                div().flex_1().min_h_0().child(
                    h_resizable("debug-split")
                        .with_state(&self.split)
                        .child(
                            resizable_panel()
                                .size(px(260.))
                                .size_range(px(160.)..px(520.))
                                .child(
                                    div()
                                        .id("debug-stack")
                                        .size_full()
                                        .bg(theme::panel())
                                        .overflow_y_scrollbar()
                                        .child(self.render_call_stack(cx))
                                        .child(self.render_watches(cx))
                                        .child(self.render_breakpoints(cx))
                                        .child(self.render_exception_filters(cx)),
                                ),
                        )
                        .child(resizable_panel().child(self.render_variables(cx)))
                        .child(
                            resizable_panel()
                                .size(px(380.))
                                .size_range(px(220.)..px(900.))
                                .child(
                                    div()
                                        .size_full()
                                        .flex()
                                        .flex_col()
                                        .border_l_1()
                                        .border_color(theme::border())
                                        .child(self.render_detail_tabs(cx))
                                        .child(detail),
                                ),
                        ),
                ),
            )
    }
}
