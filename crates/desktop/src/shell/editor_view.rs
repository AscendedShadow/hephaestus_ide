//! The editor area: breadcrumbs, gutter and the overlays drawn over the text.

use super::*;

impl IdeShell {
    pub(super) fn render_breadcrumbs(&self) -> impl IntoElement {
        let row = div()
            .flex_1()
            .min_w_0()
            .flex()
            .items_center()
            .gap_1()
            .overflow_hidden()
            .whitespace_nowrap();
        let Some(path) = self.document().path() else {
            return row
                .child(
                    div()
                        .text_color(theme::text())
                        .child(self.document().name()),
                )
                .child(
                    div()
                        .text_color(theme::subtle())
                        .child("· not saved to disk"),
                );
        };
        let relative = self
            .workspace
            .root()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path);
        let segments: Vec<String> = relative
            .iter()
            .map(|segment| segment.to_string_lossy().into_owned())
            .filter(|segment| !segment.is_empty() && segment != "\\" && segment != "/")
            .collect();
        let last = segments.len().saturating_sub(1);
        row.children(segments.into_iter().enumerate().flat_map(|(ix, segment)| {
            let separator = (ix > 0).then(|| {
                Icon::new(IconName::ChevronRight)
                    .size(px(12.))
                    .flex_shrink_0()
                    .text_color(theme::subtle())
                    .into_any_element()
            });
            let label = div()
                .flex_shrink_0()
                .when(ix == last, |label| label.text_color(theme::text()))
                .child(segment)
                .into_any_element();
            separator.into_iter().chain([label])
        }))
    }

    pub(super) fn line_number_width(&self, window: &Window, cx: &App) -> Pixels {
        let digits = match self.editor().read(cx).text().lines_len() {
            0..=9999 => 5,
            10000..=99999 => 6,
            100000..=999999 => 7,
            _ => 8,
        };
        let sample = "+".repeat(digits);
        let run = TextRun {
            len: sample.len(),
            font: theme::monospace_font(),
            color: theme::text().into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line =
            window
                .text_system()
                .shape_line(sample.into(), self.editor_text_size(), &[run], None);
        line.width + LINE_NUMBER_PADDING
    }

    pub(super) fn render_brace_guide(&self, cx: &App) -> impl IntoElement {
        let editor = self.editor().clone();
        let state = editor.read(cx);
        // Brace pairs need a full scan; cache them per editor text revision so cursor
        // moves and redraws reuse them.
        let key = (editor.entity_id(), state.text_revision());
        let pairs = {
            let mut cache = self.brace_pairs_cache.borrow_mut();
            match &*cache {
                Some((cached, pairs)) if *cached == key => pairs.clone(),
                _ => {
                    let pairs = Rc::new(folding::brace_pairs(&state.text().to_string()));
                    *cache = Some((key, pairs.clone()));
                    pairs
                }
            }
        };
        let guide = BraceGuide::locate(state.text(), &pairs, state.cursor());
        let text_size = self.editor_text_size();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                if let Some(guide) = guide
                    && let Some(geometry) = editor.read(cx).text_geometry()
                {
                    guide.paint(geometry, bounds, text_size, window);
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    pub(super) fn render_whitespace(&self) -> impl IntoElement {
        let editor = self.editor().clone();
        let font = gpui::Font {
            fallbacks: theme::monospace_font().fallbacks,
            ..gpui::font(self.editor_font_family())
        };
        let size = self.editor_text_size();
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let state = editor.read(cx);
                // Cloning a rope is cheap (shared); only visible lines are read.
                let text = state.text().clone();
                if let Some(geometry) = state.text_geometry() {
                    crate::whitespace::paint(
                        &text,
                        geometry,
                        bounds,
                        font.clone(),
                        size,
                        window,
                        cx,
                    );
                }
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    pub(super) fn gutter_row(&self, y: Pixels, cx: &App) -> Option<usize> {
        let editor = self.editor().read(cx);
        let geometry = editor.text_geometry()?;
        if y < geometry.viewport.top() || y > geometry.viewport.bottom() {
            return None;
        }
        let row = ((y - geometry.origin.y) / geometry.line_height).floor();
        (row >= 0. && (row as usize) < editor.text().lines_len()).then_some(row as usize)
    }

    pub(super) fn render_debug_markers(&self, cx: &App) -> impl IntoElement {
        let editor = self.editor().clone();
        let path = self.document().path().map(Path::to_path_buf);
        let debugger = self.debugger.read(cx);
        let markers: Vec<(usize, bool)> = path
            .as_deref()
            .and_then(|path| debugger.lines(path).map(|lines| (path, lines)))
            .map(|(path, lines)| {
                lines
                    .iter()
                    .map(|line| {
                        (
                            self.display_row(*line, cx),
                            debugger.is_rejected(path, *line),
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let execution = path
            .as_deref()
            .and_then(|path| debugger.execution_line(path))
            .map(|line| self.display_row(line, cx));
        let hover = self
            .breakpoint_hover
            .filter(|row| !markers.iter().any(|(marked, _)| marked == row));
        canvas(
            |_, _, _| {},
            move |bounds, _, window, cx| {
                let Some(geometry) = editor.read(cx).text_geometry() else {
                    return;
                };
                let top = |row: usize| geometry.origin.y + geometry.line_height * row as f32;
                let visible = |row: usize| {
                    let y = top(row);
                    y + geometry.line_height > bounds.top() && y < bounds.bottom()
                };
                let center_x = bounds.left() + EDITOR_PADDING_LEFT / 2.;
                let radius = px(BREAKPOINT_RADIUS);
                let dot = |row: usize, fill: Hsla, border: Hsla, window: &mut Window| {
                    let center_y = top(row) + geometry.line_height / 2.;
                    window.paint_quad(quad(
                        Bounds::new(
                            point(center_x - radius, center_y - radius),
                            size(radius * 2., radius * 2.),
                        ),
                        radius,
                        fill,
                        px(1.5),
                        border,
                        BorderStyle::Solid,
                    ));
                };
                window.with_content_mask(Some(ContentMask { bounds }), |window| {
                    if let Some(row) = execution.filter(|row| visible(*row)) {
                        window.paint_quad(fill(
                            Bounds::new(
                                point(bounds.left(), top(row)),
                                size(bounds.size.width, geometry.line_height),
                            ),
                            theme::execution_wash(),
                        ));
                    }
                    let color = Hsla::from(theme::breakpoint());
                    if let Some(row) = hover.filter(|row| visible(*row)) {
                        dot(row, color.opacity(0.35), color.opacity(0.), window);
                    }
                    for (row, rejected) in markers.iter().copied().filter(|(row, _)| visible(*row))
                    {
                        if rejected {
                            dot(row, color.opacity(0.), color, window);
                        } else {
                            dot(row, color, color, window);
                        }
                    }
                    if let Some(row) = execution.filter(|row| visible(*row)) {
                        let center_y = top(row) + geometry.line_height / 2.;
                        let half = px(5.);
                        let mut arrow = PathBuilder::fill();
                        arrow.move_to(point(center_x - half, center_y - half));
                        arrow.line_to(point(center_x + half, center_y));
                        arrow.line_to(point(center_x - half, center_y + half));
                        arrow.close();
                        if let Ok(path) = arrow.build() {
                            window.paint_path(path, theme::execution_line());
                        }
                    }
                });
            },
        )
        .absolute()
        .top_0()
        .left_0()
        .size_full()
    }

    pub(super) fn render_editor(
        &self,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let vim_context = self.vim.as_ref().map(VimInput::key_context);
        let area = div()
            .key_context(EDITOR_CONTEXT)
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(self.render_tabs(cx));
        if self.show_diff {
            return area.child(div().flex_1().min_h_0().child(self.diff_view.clone()));
        }
        area.child(
            div()
                .h(px(28.))
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap_2()
                .pl_4()
                .pr_2()
                .bg(theme::background())
                .text_xs()
                .text_color(theme::muted())
                .child(self.render_breadcrumbs()),
        )
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .capture_any_mouse_down(cx.listener(Self::editor_mouse_down))
                .map(|editor| self.cursor_listeners(editor, cx))
                .when_some(vim_context, |editor, context| editor.key_context(context))
                .when(self.snippet_active(), |editor| {
                    editor.key_context(SNIPPET_CONTEXT)
                })
                .capture_key_down(cx.listener(Self::editor_key_down))
                .child(
                    Input::new(self.editor())
                        .h_full()
                        .w_full()
                        .pl(EDITOR_PADDING_LEFT)
                        .bordered(false)
                        .focus_bordered(false)
                        .appearance(false)
                        .disabled(self.blocked())
                        .font_family(self.editor_font_family())
                        .text_size(self.editor_text_size())
                        .bg(theme::background())
                        .text_color(theme::text()),
                )
                .children(self.render_signature_help(cx))
                .child(self.render_brace_guide(cx))
                .when(self.buffer().preferences.show_whitespace, |editor| {
                    editor.child(self.render_whitespace())
                })
                .children(self.render_secondary_cursors(cx))
                .child(self.render_debug_markers(cx))
                .child(
                    div()
                        .id("breakpoint-gutter")
                        .debug_selector(|| "breakpoint-gutter".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .w(EDITOR_PADDING_LEFT)
                        .cursor_pointer()
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            if !hovered && this.breakpoint_hover.take().is_some() {
                                cx.notify();
                            }
                        }))
                        .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                            let row = this.gutter_row(event.position.y, cx);
                            if this.breakpoint_hover != row {
                                this.breakpoint_hover = row;
                                cx.notify();
                            }
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                cx.stop_propagation();
                                if this.blocked() || this.show_diff {
                                    return;
                                }
                                if let Some(row) = this.gutter_row(event.position.y, cx) {
                                    this.toggle_breakpoint_at_row(row, cx);
                                }
                            }),
                        ),
                )
                .child(
                    div()
                        .debug_selector(|| "fold-gutter".into())
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(EDITOR_PADDING_LEFT)
                        .w(self.line_number_width(window, cx))
                        .cursor_pointer()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|_, _, window, cx| {
                                cx.defer_in(window, Self::fold_clicked_line);
                            }),
                        ),
                ),
        )
    }
}
