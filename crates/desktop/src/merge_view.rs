use std::{io, ops::Range, path::PathBuf};

use gpui::{
    AnyElement, Context, Entity, EntityInputHandler as _, EventEmitter, FontWeight, Hsla,
    IntoElement, ListHorizontalSizingBehavior, Pixels, Render, ScrollStrategy, Subscription,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, RopeExt as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    highlighter::{Diagnostic, DiagnosticSeverity},
    input::{Input, InputEvent, InputState, TabSize},
    resizable::{ResizableState, h_resizable, resizable_panel, v_resizable},
};
use ide_core::{
    document::Document,
    git::{FileStatus, Repository},
    merge::{Choice, Merge, Segment, Side},
};

use crate::{syntax, theme, ui};

#[cfg(test)]
#[path = "merge_view_tests.rs"]
pub(crate) mod tests;

const ROW_HEIGHT: f32 = 22.;
const TEXT_SIZE: f32 = 13.;
const TAB_WIDTH: usize = 4;
const INDENT_WIDTH: usize = 2;
const PANES: [Side; 2] = [Side::Incoming, Side::Current];

pub enum MergeEvent {
    Resolved(String),
}

enum Notice {
    Info(String),
    Error(String),
}

enum PaneRow {
    Header(usize),
    Line {
        number: usize,
        text: String,
        conflict: Option<usize>,
    },
}

struct Pane {
    side: Side,
    rows: Vec<PaneRow>,
    widest: usize,
    scroll: UniformListScrollHandle,
}

impl Pane {
    fn new(side: Side, merge: &Merge) -> Self {
        let mut rows = Vec::new();
        let mut number = 0;
        let mut push = |rows: &mut Vec<PaneRow>, text: &str, conflict: Option<usize>| {
            for line in text.split_inclusive('\n') {
                number += 1;
                rows.push(PaneRow::Line {
                    number,
                    text: expand_tabs(line.trim_end_matches(['\n', '\r'])),
                    conflict,
                });
            }
        };
        for segment in merge.segments() {
            match *segment {
                Segment::Common(ref text) => push(&mut rows, text, None),
                Segment::Conflict(ix) => {
                    rows.push(PaneRow::Header(ix));
                    push(&mut rows, merge.conflicts()[ix].side(side), Some(ix));
                }
            }
        }
        let widest = (0..rows.len())
            .max_by_key(|&ix| match &rows[ix] {
                PaneRow::Line { text, .. } => text.chars().count(),
                PaneRow::Header(_) => 0,
            })
            .unwrap_or(0);
        Self {
            side,
            rows,
            widest,
            scroll: UniformListScrollHandle::new(),
        }
    }

    fn header_row(&self, conflict: usize) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| matches!(row, PaneRow::Header(ix) if *ix == conflict))
    }
}

fn expand_tabs(text: &str) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut column = 0;
    for ch in text.chars() {
        if ch == '\t' {
            let spaces = TAB_WIDTH - column % TAB_WIDTH;
            expanded.extend(std::iter::repeat_n(' ', spaces));
            column += spaces;
        } else {
            expanded.push(ch);
            column += 1;
        }
    }
    expanded
}

fn conflicts(count: usize) -> String {
    match count {
        1 => "1 conflict".into(),
        count => format!("{count} conflicts"),
    }
}

fn side_title(side: Side) -> &'static str {
    match side {
        Side::Current => "CURRENT",
        Side::Incoming => "INCOMING",
    }
}

pub struct MergeView {
    repository: Repository,
    file: FileStatus,
    path: PathBuf,
    document: Document,
    merge: Merge,
    result: Entity<InputState>,
    panes: [Pane; 2],
    active: usize,
    notice: Option<Notice>,
    busy: bool,
    vertical_split: Entity<ResizableState>,
    side_split: Entity<ResizableState>,
    _subscription: Subscription,
}

impl EventEmitter<MergeEvent> for MergeView {}

impl MergeView {
    pub fn new(
        repository: Repository,
        file: FileStatus,
        path: PathBuf,
        document: Document,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let merge = Merge::parse(&document.text().to_string());
        let result = cx.new(|cx| {
            InputState::new(window, cx)
                .code_editor(syntax::language(Some(&path)))
                .line_number(true)
                .soft_wrap(false)
                .tab_size(TabSize {
                    tab_size: INDENT_WIDTH,
                    hard_tabs: false,
                })
                .default_value(merge.text().to_string())
        });
        let subscription = cx.subscribe(&result, |this, editor, event, cx| {
            if matches!(event, InputEvent::Change) {
                let text = editor.read(cx).text().to_string();
                this.merge.sync(&text);
                this.mark_conflicts(cx);
                cx.notify();
            }
        });
        let view = Self {
            repository,
            file,
            path,
            document,
            panes: PANES.map(|side| Pane::new(side, &merge)),
            merge,
            result,
            active: 0,
            notice: None,
            busy: false,
            vertical_split: cx.new(|_| ResizableState::default()),
            side_split: cx.new(|_| ResizableState::default()),
            _subscription: subscription,
        };
        view.mark_conflicts(cx);
        view
    }

    fn sync_result(&mut self, cx: &mut Context<Self>) {
        let text = self.result.read(cx).text().to_string();
        self.merge.sync(&text);
    }

    fn mark_conflicts(&self, cx: &mut Context<Self>) {
        let ranges: Vec<Range<usize>> = (0..self.merge.conflicts().len())
            .filter(|&ix| !self.merge.choice(ix).is_resolved())
            .map(|ix| self.merge.range(ix))
            .collect();
        self.result.update(cx, |editor, cx| {
            let text = editor.text().clone();
            let diagnostics: Vec<_> = ranges
                .into_iter()
                .filter(|range| range.end <= text.len())
                .map(|range| {
                    Diagnostic::new(
                        text.offset_to_position(range.start)..text.offset_to_position(range.end),
                        "Unresolved merge conflict",
                    )
                    .with_severity(DiagnosticSeverity::Warning)
                })
                .collect();
            if let Some(set) = editor.diagnostics_mut() {
                set.reset(&text);
                set.extend(diagnostics);
            }
            cx.notify();
        });
    }

    pub(crate) fn resolve(
        &mut self,
        ix: usize,
        choice: Choice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy {
            return;
        }
        self.sync_result(cx);
        self.active = ix;
        self.notice = None;
        if let Some((range, text)) = self.merge.resolve(ix, choice) {
            self.result.update(cx, |editor, cx| {
                let current = editor.text().to_string();
                let start = current[..range.start].encode_utf16().count();
                let end = start + current[range].encode_utf16().count();
                editor.replace_text_in_range(Some(start..end), &text, window, cx);
            });
            self.mark_conflicts(cx);
        }
        cx.notify();
    }

    pub(crate) fn toggle(
        &mut self,
        ix: usize,
        side: Side,
        accept: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.sync_result(cx);
        let choice = self.merge.choice(ix).toggle(side, accept);
        self.resolve(ix, choice, window, cx);
    }

    pub(crate) fn accept_all(&mut self, side: Side, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_result(cx);
        let choice = match side {
            Side::Current => Choice::Current,
            Side::Incoming => Choice::Incoming,
        };
        for ix in 0..self.merge.conflicts().len() {
            if self.merge.choice(ix) != Choice::Custom {
                self.resolve(ix, choice, window, cx);
            }
        }
        self.active = 0;
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.merge = Merge::parse(&self.document.text().to_string());
        self.panes = PANES.map(|side| Pane::new(side, &self.merge));
        let text = self.merge.text().to_string();
        self.result
            .update(cx, |editor, cx| editor.set_value(text, window, cx));
        self.active = 0;
        self.notice = Some(Notice::Info("Reset the result to the file on disk".into()));
        self.mark_conflicts(cx);
        cx.notify();
    }

    fn navigate(&mut self, step: isize, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.merge.conflicts().len();
        if count == 0 {
            return;
        }
        self.active = (self.active as isize + step).rem_euclid(count as isize) as usize;
        self.reveal(self.active, window, cx);
    }

    fn reveal(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.active = ix;
        for pane in &self.panes {
            if let Some(row) = pane.header_row(ix) {
                pane.scroll.scroll_to_item(row, ScrollStrategy::Center);
            }
        }
        self.sync_result(cx);
        let offset = self.merge.range(ix).start;
        self.result
            .update(cx, |editor, cx| editor.reveal(offset, window, cx));
        cx.notify();
    }

    pub(crate) fn save(&mut self, cx: &mut Context<Self>) {
        self.write(false, cx);
    }

    pub(crate) fn complete(&mut self, cx: &mut Context<Self>) {
        self.write(true, cx);
    }

    fn write(&mut self, stage: bool, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.sync_result(cx);
        if stage {
            let remaining = self.merge.unresolved();
            let leftover = Merge::parse(self.merge.text()).conflicts().len();
            if remaining > 0 {
                self.notice = Some(Notice::Error(format!(
                    "Resolve the remaining {} before completing the merge",
                    conflicts(remaining)
                )));
                cx.notify();
                return;
            }
            if leftover > 0 {
                self.notice = Some(Notice::Error(
                    "The result still contains conflict markers".into(),
                ));
                cx.notify();
                return;
            }
        }
        let mut document = self.document.clone();
        document.set_text(self.merge.text().into());
        let (path, repository, file) = (
            self.path.clone(),
            self.repository.clone(),
            self.file.clone(),
        );
        self.run(
            move || {
                let document = document.save_to(&path)?;
                if stage {
                    repository.stage(&[file])?;
                }
                Ok(Some(document))
            },
            stage,
            cx,
        );
    }

    fn mark_resolved(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let (repository, file) = (self.repository.clone(), self.file.clone());
        self.run(move || repository.stage(&[file]).map(|()| None), true, cx);
    }

    fn run(
        &mut self,
        operation: impl FnOnce() -> io::Result<Option<Document>> + Send + 'static,
        resolved: bool,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.notice = None;
        cx.notify();
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            let result = executor.spawn(async move { operation() }).await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match result {
                    Ok(document) => {
                        if let Some(document) = document {
                            this.document = document;
                        }
                        if resolved {
                            let summary = format!("Resolved conflicts in {}", this.file.relative);
                            this.notice = Some(Notice::Info(summary.clone()));
                            cx.emit(MergeEvent::Resolved(summary));
                        } else {
                            this.notice = Some(Notice::Info("Saved the merge result".into()));
                        }
                    }
                    Err(error) => this.notice = Some(Notice::Error(error.to_string())),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.merge.conflicts().len();
        let navigation = total > 0;
        div()
            .h(px(32.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .pl_4()
            .pr_2()
            .border_b_1()
            .border_color(theme::border())
            .text_xs()
            .text_color(theme::muted())
            .child(
                Icon::new(IconName::File)
                    .size(px(14.))
                    .flex_shrink_0()
                    .text_color(theme::subtle()),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_color(theme::text())
                    .child(self.file.relative.clone()),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .px_1p5()
                    .rounded_sm()
                    .bg(
                        Hsla::from(theme::git_change(ide_core::git::Change::Conflicted))
                            .opacity(0.16),
                    )
                    .text_color(theme::git_change(ide_core::git::Change::Conflicted))
                    .text_size(px(10.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("MERGING"),
            )
            .child(div().flex_1())
            .when(navigation, |bar| {
                bar.child(
                    div()
                        .debug_selector(|| "merge-position".into())
                        .flex_shrink_0()
                        .child(format!("Conflict {} of {total}", self.active + 1)),
                )
                .child(
                    Button::new("merge-previous")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronUp)
                        .tooltip("Previous Conflict")
                        .on_click(cx.listener(|this, _, window, cx| this.navigate(-1, window, cx))),
                )
                .child(
                    Button::new("merge-next")
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronDown)
                        .tooltip("Next Conflict")
                        .on_click(cx.listener(|this, _, window, cx| this.navigate(1, window, cx))),
                )
                .child(div().w(px(1.)).h(px(16.)).bg(theme::border()))
                .child(
                    Button::new("merge-accept-all-incoming")
                        .ghost()
                        .xsmall()
                        .label("Accept All Incoming")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.accept_all(Side::Incoming, window, cx)
                        })),
                )
                .child(
                    Button::new("merge-accept-all-current")
                        .ghost()
                        .xsmall()
                        .label("Accept All Current")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.accept_all(Side::Current, window, cx)
                        })),
                )
                .child(
                    Button::new("merge-reset")
                        .ghost()
                        .xsmall()
                        .icon(IconName::Undo2)
                        .tooltip("Reset Result")
                        .disabled(self.busy)
                        .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                )
            })
    }

    fn render_pane(&self, pane_ix: usize, cx: &mut Context<Self>) -> impl IntoElement {
        let pane = &self.panes[pane_ix];
        let side = pane.side;
        let label = self.merge.conflicts().first().map(|conflict| match side {
            Side::Current => conflict.current_label.clone(),
            Side::Incoming => conflict.incoming_label.clone(),
        });
        div()
            .size_full()
            .flex()
            .flex_col()
            .when(pane_ix > 0, |pane| {
                pane.border_l_1().border_color(theme::border())
            })
            .child(
                div()
                    .h(px(26.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_3()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(theme::border())
                    .child(
                        div()
                            .size(px(8.))
                            .rounded_full()
                            .bg(theme::merge_label(side)),
                    )
                    .child(ui::caption(side_title(side)))
                    .children(label.filter(|label| !label.is_empty()).map(|label| {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme::muted())
                            .child(label)
                    })),
            )
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        ("merge-pane", pane_ix),
                        pane.rows.len(),
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            range
                                .map(|ix| this.render_pane_row(pane_ix, ix, cx))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(Some(pane.widest))
                    .track_scroll(pane.scroll.clone()),
                ),
            )
    }

    fn render_pane_row(&self, pane_ix: usize, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let side = self.panes[pane_ix].side;
        let row = div()
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .whitespace_nowrap();
        match self.panes[pane_ix].rows[ix] {
            PaneRow::Header(conflict) => self.render_header(row, side, conflict, cx),
            PaneRow::Line {
                number,
                ref text,
                conflict,
            } => {
                let accepted = conflict.is_some_and(|ix| self.merge.choice(ix).accepts(side));
                row.font(theme::monospace_font())
                    .text_size(px(TEXT_SIZE))
                    .when(conflict.is_some(), |row| row.bg(theme::merge_side(side)))
                    .child(
                        div()
                            .w(px(44.))
                            .flex_shrink_0()
                            .pr_2()
                            .text_right()
                            .text_color(theme::subtle())
                            .child(number.to_string()),
                    )
                    .child(
                        div()
                            .w(px(3.))
                            .h_full()
                            .flex_shrink_0()
                            .mr_2()
                            .when(accepted, |bar| bar.bg(theme::merge_label(side))),
                    )
                    .child(div().pr_4().text_color(theme::text()).child(text.clone()))
                    .into_any_element()
            }
        }
    }

    fn render_header(
        &self,
        row: gpui::Div,
        side: Side,
        conflict: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let choice = self.merge.choice(conflict);
        let (accept, both, [id, check_id, both_id]) = match side {
            Side::Current => (
                "Accept Current",
                "Accept Both (Current First)",
                [
                    "merge-current",
                    "merge-current-accept",
                    "merge-current-both",
                ],
            ),
            Side::Incoming => (
                "Accept Incoming",
                "Accept Both (Incoming First)",
                [
                    "merge-incoming",
                    "merge-incoming-accept",
                    "merge-incoming-both",
                ],
            ),
        };
        let view = cx.entity().downgrade();
        let status = match choice {
            Choice::Unresolved => (
                "Unresolved",
                theme::git_change(ide_core::git::Change::Conflicted),
            ),
            Choice::Custom => ("Edited", theme::accent()),
            _ => ("Resolved", theme::merge_label(Side::Current)),
        };
        row.id((id, conflict))
            .gap_3()
            .px_3()
            .text_xs()
            .bg(theme::panel())
            .when(conflict == self.active, |row| {
                row.border_l_2().border_color(theme::accent())
            })
            .child(
                Checkbox::new((check_id, conflict))
                    .xsmall()
                    .label(accept)
                    .checked(choice.accepts(side))
                    .disabled(self.busy)
                    .on_click(move |checked, window, cx| {
                        let checked = *checked;
                        let _ = view.update(cx, |view, cx| {
                            view.toggle(conflict, side, checked, window, cx)
                        });
                    }),
            )
            .child(
                div()
                    .id((both_id, conflict))
                    .flex_shrink_0()
                    .cursor_pointer()
                    .text_color(theme::muted())
                    .hover(|style| style.text_color(theme::text()))
                    .child(both)
                    .on_click(cx.listener(move |this, _, window, cx| {
                        cx.stop_propagation();
                        this.resolve(conflict, Choice::both(side), window, cx)
                    })),
            )
            .child(div().flex_1())
            .child(div().flex_shrink_0().text_color(status.1).child(status.0))
            .on_click(cx.listener(move |this, _, window, cx| this.reveal(conflict, window, cx)))
            .into_any_element()
    }

    fn render_result(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let remaining = self.merge.unresolved();
        let (status, color) = if remaining == 0 {
            (
                "All conflicts resolved".to_string(),
                theme::merge_label(Side::Current),
            )
        } else {
            (
                format!("{} remaining", conflicts(remaining)),
                theme::git_change(ide_core::git::Change::Conflicted),
            )
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .border_t_1()
            .border_color(theme::border())
            .child(
                div()
                    .h(px(30.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_3()
                    .pr_2()
                    .bg(theme::panel())
                    .border_b_1()
                    .border_color(theme::border())
                    .child(ui::caption("RESULT"))
                    .child(
                        div()
                            .debug_selector(|| "merge-remaining".into())
                            .text_xs()
                            .text_color(color)
                            .child(status),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("merge-save")
                            .ghost()
                            .xsmall()
                            .label("Save")
                            .tooltip("Write the result without marking the conflict resolved")
                            .disabled(self.busy)
                            .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
                    )
                    .child(
                        Button::new("merge-complete")
                            .primary()
                            .xsmall()
                            .icon(IconName::Check)
                            .label("Complete Merge")
                            .disabled(self.busy || remaining > 0)
                            .on_click(cx.listener(|this, _, _, cx| this.complete(cx))),
                    ),
            )
            .child(
                div().flex_1().min_h_0().child(
                    Input::new(&self.result)
                        .h_full()
                        .w_full()
                        .bordered(false)
                        .focus_bordered(false)
                        .appearance(false)
                        .disabled(self.busy)
                        .font_family(theme::mono_family())
                        .text_size(px(TEXT_SIZE))
                        .bg(theme::background())
                        .text_color(theme::text()),
                ),
            )
    }

    fn render_without_markers(&self, cx: &mut Context<Self>) -> impl IntoElement {
        ui::empty_state(
            Icon::new(IconName::TriangleAlert).size(px(24.)),
            "No conflict markers were found in this file.",
        )
        .pt_4()
        .child(
            div()
                .text_xs()
                .text_color(theme::muted())
                .child("Edit the file directly, then mark it as resolved."),
        )
        .child(
            div().pt_2().child(
                Button::new("merge-mark-resolved")
                    .primary()
                    .small()
                    .label("Mark as Resolved")
                    .disabled(self.busy)
                    .on_click(cx.listener(|this, _, _, cx| this.mark_resolved(cx))),
            ),
        )
    }
}

impl Render for MergeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let notice = self.notice.as_ref().map(|notice| {
            let (text, color) = match notice {
                Notice::Info(text) => (text.clone(), theme::muted()),
                Notice::Error(text) => (text.clone(), theme::error()),
            };
            div()
                .debug_selector(|| "merge-notice".into())
                .flex_shrink_0()
                .px_4()
                .py_1()
                .text_xs()
                .text_color(color)
                .border_b_1()
                .border_color(theme::border())
                .child(text)
        });
        let body = if self.merge.conflicts().is_empty() {
            self.render_without_markers(cx).into_any_element()
        } else {
            v_resizable("merge-split")
                .with_state(&self.vertical_split)
                .child(
                    resizable_panel().child(
                        h_resizable("merge-sides")
                            .with_state(&self.side_split)
                            .child(resizable_panel().child(self.render_pane(0, cx)))
                            .child(resizable_panel().child(self.render_pane(1, cx))),
                    ),
                )
                .child(
                    resizable_panel()
                        .size(px(300.))
                        .size_range(px(120.)..Pixels::MAX)
                        .child(self.render_result(cx)),
                )
                .into_any_element()
        };
        div()
            .debug_selector(|| "merge-view".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .child(self.render_toolbar(cx))
            .children(notice)
            .child(div().flex_1().min_h_0().child(body))
    }
}
