use std::ops::Range;

use gpui::{
    App, Context, Div, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    ListHorizontalSizingBehavior, Render, SharedString, Subscription, Task,
    UniformListScrollHandle, Window, div, prelude::*, px, uniform_list,
};
use gpui_component::{Icon, IconName};
use ide_core::{
    document::Document,
    git::{self, Change, Diff, DiffLine, FileStatus, LineKind, Repository},
};

use crate::{
    commands::SaveFile,
    git_panel::Selection,
    merge_view::{MergeEvent, MergeView},
    theme,
};

const ROW_HEIGHT: f32 = 20.;

pub enum DiffViewEvent {
    Resolved(String),
}

struct MergeSource {
    target: Selection,
    repository: Repository,
    file: FileStatus,
    document: Result<Document, String>,
}

pub struct DiffView {
    target: Option<Selection>,
    diff: Option<(Selection, Result<Diff, String>)>,
    conflicted: bool,
    merge_source: Option<MergeSource>,
    merge: Option<(Selection, Entity<MergeView>, Subscription)>,
    widest_line: usize,
    scroll: UniformListScrollHandle,
    task: Option<Task<()>>,
    focus_handle: FocusHandle,
}

impl EventEmitter<DiffViewEvent> for DiffView {}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl DiffView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            target: None,
            diff: None,
            conflicted: false,
            merge_source: None,
            merge: None,
            widest_line: 0,
            scroll: UniformListScrollHandle::new(),
            task: None,
            focus_handle: cx.focus_handle(),
        }
    }

    pub(crate) fn target(&self) -> Option<&Selection> {
        self.target.as_ref()
    }

    pub(crate) fn loaded(&self) -> Option<&Result<Diff, String>> {
        match (&self.target, &self.diff) {
            (Some(target), Some((shown, diff))) if target == shown => Some(diff),
            _ => None,
        }
    }

    pub(crate) fn merge_view(&self) -> Option<&Entity<MergeView>> {
        match (&self.target, &self.merge) {
            (Some(target), Some((shown, view, _))) if target == shown => Some(view),
            _ => None,
        }
    }

    pub fn title(&self) -> Option<String> {
        self.target.as_ref().map(|target| {
            let name = target
                .relative
                .rsplit_once('/')
                .map_or(target.relative.as_str(), |(_, name)| name);
            let side = if self.conflicted {
                "Merging"
            } else if target.staged {
                "Staged"
            } else {
                "Unstaged"
            };
            format!("{name} ({side})")
        })
    }

    pub(crate) fn load(
        &mut self,
        repository: Repository,
        target: Selection,
        file: FileStatus,
        cx: &mut Context<Self>,
    ) {
        if !target.staged && file.unstaged == Some(Change::Conflicted) {
            self.load_merge(repository, target, file, cx);
            return;
        }
        self.conflicted = false;
        self.merge_source = None;
        self.merge = None;
        self.target = Some(target.clone());
        let executor = cx.background_executor().clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let staged = target.staged;
            let diff = executor
                .spawn(async move { repository.diff(&file, staged) })
                .await
                .map_err(|error| error.to_string());
            let _ = this.update(cx, |this, cx| {
                let widest = diff.as_ref().ok().and_then(|diff| {
                    (0..diff.lines.len()).max_by_key(|&ix| diff.lines[ix].text.chars().count())
                });
                if this.diff.as_ref().is_none_or(|(shown, _)| *shown != target) {
                    this.scroll = UniformListScrollHandle::new();
                }
                this.widest_line = widest.unwrap_or(0);
                this.diff = Some((target, diff));
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn load_merge(
        &mut self,
        repository: Repository,
        target: Selection,
        file: FileStatus,
        cx: &mut Context<Self>,
    ) {
        let loaded = self.conflicted
            && self.target.as_ref() == Some(&target)
            && (self.merge.is_some()
                || self
                    .merge_source
                    .as_ref()
                    .is_none_or(|source| source.document.is_ok()));
        if loaded {
            return;
        }
        self.target = Some(target.clone());
        self.conflicted = true;
        self.diff = None;
        self.merge = None;
        self.merge_source = None;
        let executor = cx.background_executor().clone();
        let path = git::absolute(repository.root(), &target.relative);
        self.task = Some(cx.spawn(async move |this, cx| {
            let document = executor
                .spawn(async move { Document::open(&path) })
                .await
                .map_err(|error| error.to_string());
            let _ = this.update(cx, |this, cx| {
                if this.target.as_ref() == Some(&target) && this.conflicted {
                    this.merge_source = Some(MergeSource {
                        target,
                        repository,
                        file,
                        document,
                    });
                    cx.notify();
                }
            });
        }));
        cx.notify();
    }

    fn open_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(source) = self.merge_source.take_if(|source| source.document.is_ok()) else {
            return;
        };
        let Ok(document) = source.document else {
            return;
        };
        let path = git::absolute(source.repository.root(), &source.target.relative);
        let view =
            cx.new(|cx| MergeView::new(source.repository, source.file, path, document, window, cx));
        let subscription = cx.subscribe(&view, |_, _, event, cx| match event {
            MergeEvent::Resolved(summary) => cx.emit(DiffViewEvent::Resolved(summary.clone())),
        });
        self.merge = Some((source.target, view, subscription));
    }

    fn save(&mut self, _: &SaveFile, _: &mut Window, cx: &mut Context<Self>) {
        match self.merge_view().cloned() {
            Some(view) => view.update(cx, |view, cx| view.save(cx)),
            None => cx.propagate(),
        }
    }

    pub(crate) fn clear(&mut self, cx: &mut Context<Self>) {
        self.target = None;
        self.diff = None;
        self.conflicted = false;
        self.merge_source = None;
        self.merge = None;
        self.task = None;
        cx.notify();
    }
}

fn render_diff_line(line: &DiffLine) -> Div {
    let (background, sign, color, sign_color) = match line.kind {
        LineKind::Added => (
            Some(theme::diff_added()),
            "+",
            theme::text(),
            theme::git_change(Change::Added),
        ),
        LineKind::Removed => (
            Some(theme::diff_removed()),
            "-",
            theme::text(),
            theme::git_change(Change::Untracked),
        ),
        LineKind::Hunk => (
            Some(theme::diff_hunk()),
            "",
            theme::accent(),
            theme::accent(),
        ),
        LineKind::Context => (None, "", theme::text(), theme::text()),
        LineKind::Meta | LineKind::Note => (None, "", theme::muted(), theme::muted()),
    };
    let number = |number: Option<u32>| {
        div()
            .w(px(44.))
            .flex_shrink_0()
            .pr_2()
            .text_right()
            .text_color(theme::subtle())
            .child(number.map(|number| number.to_string()).unwrap_or_default())
    };
    div()
        .w_full()
        .h(px(ROW_HEIGHT))
        .flex()
        .items_center()
        .whitespace_nowrap()
        .font(theme::monospace_font())
        .text_size(px(13.))
        .when_some(background, |row, background| row.bg(background))
        .child(number(line.old_line))
        .child(number(line.new_line))
        .child(
            div()
                .w(px(16.))
                .flex_shrink_0()
                .text_color(sign_color)
                .child(sign),
        )
        .child(div().pr_4().text_color(color).child(line.text.clone()))
}

impl Render for DiffView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.conflicted {
            self.open_merge(window, cx);
            let body = match (self.merge_view(), &self.merge_source) {
                (Some(view), _) => view.clone().into_any_element(),
                (
                    None,
                    Some(MergeSource {
                        document: Err(error),
                        ..
                    }),
                ) => div()
                    .p_3()
                    .text_color(theme::error())
                    .child(error.clone())
                    .into_any_element(),
                _ => div()
                    .p_3()
                    .text_xs()
                    .text_color(theme::muted())
                    .child("Loading merge conflicts…")
                    .into_any_element(),
            };
            return div()
                .debug_selector(|| "git-diff".into())
                .track_focus(&self.focus_handle)
                .on_action(cx.listener(Self::save))
                .size_full()
                .bg(theme::background())
                .child(body)
                .into_any_element();
        }
        let message = |text: SharedString| {
            div()
                .p_3()
                .text_xs()
                .text_color(theme::muted())
                .child(text)
                .into_any_element()
        };
        let body = match self.loaded() {
            None if self.target.is_none() => message("Select a file to see its changes.".into()),
            None => message("Loading diff…".into()),
            Some(Err(error)) => div()
                .p_3()
                .text_color(theme::error())
                .child(error.clone())
                .into_any_element(),
            Some(Ok(diff)) if diff.lines.is_empty() => message("No changes to show.".into()),
            Some(Ok(diff)) => div()
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list(
                        "git-diff",
                        diff.lines.len(),
                        cx.processor(|this, range: Range<usize>, _, _| {
                            let Some((_, Ok(diff))) = &this.diff else {
                                return Vec::new();
                            };
                            range
                                .filter_map(|ix| diff.lines.get(ix))
                                .map(render_diff_line)
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .with_horizontal_sizing_behavior(ListHorizontalSizingBehavior::Unconstrained)
                    .with_width_from_item(Some(self.widest_line))
                    .track_scroll(self.scroll.clone()),
                )
                .into_any_element(),
        };
        div()
            .debug_selector(|| "git-diff".into())
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::background())
            .when_some(self.target.as_ref(), |view, target| {
                view.child(
                    div()
                        .h(px(28.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .gap_2()
                        .pl_4()
                        .pr_2()
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
                                .child(target.relative.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .px_1p5()
                                .rounded_sm()
                                .bg(theme::accent_wash())
                                .text_color(theme::accent())
                                .text_size(px(10.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(if target.staged { "STAGED" } else { "UNSTAGED" }),
                        ),
                )
            })
            .child(body)
            .into_any_element()
    }
}
