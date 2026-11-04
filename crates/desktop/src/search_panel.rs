//! The project Search panel (IDE-003) with workspace replace preview (IDE-004).

use std::{
    collections::HashSet,
    ops::Range,
    path::{Path, PathBuf},
};

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, IntoElement,
    KeyDownEvent, Render, SharedString, UniformListScrollHandle, Window, div, prelude::*, px,
    uniform_list,
};
use gpui_component::{
    Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    input::{Input, InputEvent, InputState},
};
use ide_core::project_search::{self, Hit, Options, Results};

use crate::{theme, ui};

pub const CONTEXT: &str = "SearchPanel";
const ROW_HEIGHT: f32 = 22.;

pub enum SearchPanelEvent {
    /// The query or options changed; run the search again.
    Search,
    Open(Hit),
    /// Replace the selected hits.
    Replace,
    /// Ask for more results than the current limit.
    LoadMore,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    File { path: PathBuf, count: usize },
    Hit(usize),
}

pub struct SearchPanel {
    pub query: Entity<InputState>,
    pub replace: Entity<InputState>,
    pub include: Entity<InputState>,
    pub exclude: Entity<InputState>,
    pub case_sensitive: bool,
    pub whole_word: bool,
    pub regex: bool,
    pub use_ignore_files: bool,
    pub show_replace: bool,
    pub show_filters: bool,
    pub scope: Option<PathBuf>,
    pub root: Option<PathBuf>,
    pub limit: usize,
    results: Results,
    error: Option<String>,
    pub searching: bool,
    /// Hits excluded from replacement, by index into `results.hits`.
    deselected: HashSet<usize>,
    collapsed: HashSet<PathBuf>,
    selected: usize,
    pub report: Option<String>,
    scroll: UniformListScrollHandle,
    focus_handle: FocusHandle,
    _subscriptions: Vec<gpui::Subscription>,
}

impl EventEmitter<SearchPanelEvent> for SearchPanel {}

impl Focusable for SearchPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SearchPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = |placeholder: &'static str, window: &mut Window, cx: &mut Context<Self>| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .validate(|text, _| !text.contains(['\n', '\r']))
            })
        };
        let query = input("Search", window, cx);
        let replace = input("Replace", window, cx);
        let include = input("files to include, e.g. *.rs, src/**", window, cx);
        let exclude = input("files to exclude, e.g. tests/**", window, cx);
        let mut subscriptions = Vec::new();
        for (ix, field) in [&query, &include, &exclude, &replace]
            .into_iter()
            .enumerate()
        {
            subscriptions.push(
                cx.subscribe_in(field, window, move |this, _, event, _, cx| match event {
                    InputEvent::Change if ix < 3 => {
                        this.limit = project_search::DEFAULT_MATCH_LIMIT;
                        cx.emit(SearchPanelEvent::Search);
                    }
                    InputEvent::Change => cx.notify(),
                    InputEvent::PressEnter { .. } if ix == 3 => cx.emit(SearchPanelEvent::Replace),
                    InputEvent::PressEnter { .. } => this.open_from_query(cx),
                    _ => {}
                }),
            );
        }
        Self {
            query,
            replace,
            include,
            exclude,
            case_sensitive: false,
            whole_word: false,
            regex: false,
            use_ignore_files: true,
            show_replace: false,
            show_filters: false,
            scope: None,
            root: None,
            limit: project_search::DEFAULT_MATCH_LIMIT,
            results: Results::default(),
            error: None,
            searching: false,
            deselected: HashSet::new(),
            collapsed: HashSet::new(),
            selected: 0,
            report: None,
            scroll: UniformListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn options(&self, cx: &App) -> Options {
        Options {
            query: self.query.read(cx).value().to_string(),
            case_sensitive: self.case_sensitive,
            whole_word: self.whole_word,
            regex: self.regex,
            include: self.include.read(cx).value().to_string(),
            exclude: self.exclude.read(cx).value().to_string(),
            use_ignore_files: self.use_ignore_files,
            scope: self.scope.clone(),
        }
    }

    pub fn replacement(&self, cx: &App) -> String {
        self.replace.read(cx).value().to_string()
    }

    pub fn results(&self) -> &Results {
        &self.results
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn set_results(&mut self, results: Results, cx: &mut Context<Self>) {
        self.results = results;
        self.error = None;
        self.searching = false;
        self.deselected.clear();
        self.selected = self.selected.min(self.rows().len().saturating_sub(1));
        cx.notify();
    }

    pub fn set_error(&mut self, error: Option<String>, cx: &mut Context<Self>) {
        self.error = error;
        self.searching = false;
        if self.error.is_some() {
            self.results = Results::default();
        }
        cx.notify();
    }

    pub fn set_scope(&mut self, scope: Option<PathBuf>, cx: &mut Context<Self>) {
        self.scope = scope;
        cx.emit(SearchPanelEvent::Search);
        cx.notify();
    }

    pub fn focus_query(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.query.update(cx, |query, cx| query.focus(window, cx));
    }

    /// Hits selected for replacement, grouped by file.
    pub fn selected_hits(&self) -> Vec<(PathBuf, Vec<Hit>)> {
        project_search::group(&self.results.hits)
            .into_iter()
            .filter_map(|(path, indices)| {
                let hits: Vec<Hit> = indices
                    .into_iter()
                    .filter(|ix| !self.deselected.contains(ix))
                    .map(|ix| self.results.hits[ix].clone())
                    .collect();
                (!hits.is_empty()).then_some((path, hits))
            })
            .collect()
    }

    pub fn set_hit_selected(&mut self, ix: usize, selected: bool, cx: &mut Context<Self>) {
        if selected {
            self.deselected.remove(&ix);
        } else {
            self.deselected.insert(ix);
        }
        cx.notify();
    }

    pub fn set_file_selected(&mut self, path: &Path, selected: bool, cx: &mut Context<Self>) {
        let indices: Vec<usize> = self
            .results
            .hits
            .iter()
            .enumerate()
            .filter(|(_, hit)| hit.path == path)
            .map(|(ix, _)| ix)
            .collect();
        for ix in indices {
            self.set_hit_selected(ix, selected, cx);
        }
    }

    pub fn set_all_selected(&mut self, selected: bool, cx: &mut Context<Self>) {
        if selected {
            self.deselected.clear();
        } else {
            self.deselected = (0..self.results.hits.len()).collect();
        }
        cx.notify();
    }

    fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (path, indices) in project_search::group(&self.results.hits) {
            let collapsed = self.collapsed.contains(&path);
            rows.push(Row::File {
                count: indices.len(),
                path,
            });
            if !collapsed {
                rows.extend(indices.into_iter().map(Row::Hit));
            }
        }
        rows
    }

    pub fn hit_count(&self) -> usize {
        self.results.hits.len()
    }

    /// Select the row of the `n`th hit (for keyboard tests and next/previous result).
    pub fn select_hit(&mut self, n: usize, cx: &mut Context<Self>) {
        if let Some(row) = self.rows().iter().position(|row| *row == Row::Hit(n)) {
            self.selected = row;
            self.scroll
                .scroll_to_item(row, gpui::ScrollStrategy::Center);
            cx.notify();
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows();
        let row = rows
            .get(self.selected)
            .or_else(|| rows.iter().find(|row| matches!(row, Row::Hit(_))));
        match row.cloned() {
            Some(Row::Hit(ix)) => cx.emit(SearchPanelEvent::Open(self.results.hits[ix].clone())),
            Some(Row::File { path, .. }) => {
                if !self.collapsed.remove(&path) {
                    self.collapsed.insert(path);
                }
                cx.notify();
            }
            None => {}
        }
    }

    /// Enter in the query field opens the selected result, or the first result after
    /// the selected file header.
    fn open_from_query(&mut self, cx: &mut Context<Self>) {
        let rows = self.rows();
        let hit = rows
            .iter()
            .skip(self.selected)
            .chain(rows.iter())
            .find_map(|row| match row {
                Row::Hit(ix) => Some(*ix),
                Row::File { .. } => None,
            });
        if let Some(ix) = hit {
            cx.emit(SearchPanelEvent::Open(self.results.hits[ix].clone()));
        }
    }

    fn move_selection(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.rows().len();
        if len == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, len as isize - 1) as usize;
        self.scroll
            .scroll_to_item(self.selected, gpui::ScrollStrategy::Center);
        cx.notify();
    }

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event.keystroke.key.as_str() {
            "down" => self.move_selection(1, cx),
            "up" => self.move_selection(-1, cx),
            "pagedown" => self.move_selection(10, cx),
            "pageup" => self.move_selection(-10, cx),
            "enter" if self.focus_handle.is_focused(window) => self.open_selected(cx),
            "space" if self.focus_handle.is_focused(window) => {
                if let Some(Row::Hit(ix)) = self.rows().get(self.selected).cloned() {
                    let selected = self.deselected.contains(&ix);
                    self.set_hit_selected(ix, selected, cx);
                }
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn toggle(
        &self,
        id: &'static str,
        label: &'static str,
        tooltip: &'static str,
        on: bool,
        set: fn(&mut Self, bool),
        cx: &mut Context<Self>,
    ) -> Button {
        Button::new(id)
            .ghost()
            .xsmall()
            .label(label)
            .tooltip(tooltip)
            .selected(on)
            .on_click(cx.listener(move |this, _, _, cx| {
                set(this, !on);
                this.limit = project_search::DEFAULT_MATCH_LIMIT;
                cx.emit(SearchPanelEvent::Search);
                cx.notify();
            }))
    }

    fn summary(&self) -> String {
        let hits = self.results.hits.len();
        let files = project_search::group(&self.results.hits).len();
        let mut text = format!(
            "{hits} result{} in {files} file{}",
            if hits == 1 { "" } else { "s" },
            if files == 1 { "" } else { "s" }
        );
        if self.results.truncated {
            text.push_str(&format!(" — limited to the first {}", self.limit));
        }
        if self.results.files_truncated {
            text.push_str(" — file limit reached, some files were not searched");
        }
        text
    }

    fn render_row(&self, row: &Row, ix: usize, cx: &mut Context<Self>) -> gpui::AnyElement {
        let selected = ix == self.selected;
        let base = div()
            .id(("search-row", ix))
            .h(px(ROW_HEIGHT))
            .w_full()
            .flex()
            .items_center()
            .gap_1p5()
            .pr_2()
            .rounded_sm()
            .cursor_pointer()
            .when(selected, |row| row.bg(theme::active_row()))
            .hover(|style| style.bg(theme::hover()));
        let replace = self.show_replace;
        match row {
            Row::File { path, count } => {
                let collapsed = self.collapsed.contains(path);
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                let folder = self
                    .root
                    .as_deref()
                    .and_then(|root| path.parent()?.strip_prefix(root).ok())
                    .map(|folder| folder.display().to_string())
                    .unwrap_or_default();
                let all_selected = self
                    .results
                    .hits
                    .iter()
                    .enumerate()
                    .filter(|(_, hit)| hit.path == *path)
                    .all(|(ix, _)| !self.deselected.contains(&ix));
                let toggle_path = path.clone();
                let click_path = path.clone();
                base.pl_1()
                    .child(
                        Icon::new(if collapsed {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(12.))
                        .text_color(theme::subtle()),
                    )
                    .when(replace, |row| {
                        row.child(
                            Checkbox::new(("search-file-check", ix))
                                .checked(all_selected)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.set_file_selected(&toggle_path, *checked, cx)
                                })),
                        )
                    })
                    .child(div().font_weight(FontWeight::MEDIUM).child(name))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme::subtle())
                            .child(folder),
                    )
                    .child(ui::count_badge(*count))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        if !this.collapsed.remove(&click_path) {
                            this.collapsed.insert(click_path.clone());
                        }
                        cx.notify();
                    }))
                    .into_any_element()
            }
            Row::Hit(hit_ix) => {
                let hit = &self.results.hits[*hit_ix];
                let range = hit.range_in_line();
                let line = &hit.line_text;
                let lead = line[..range.start].trim_start();
                let lead: String = if lead.chars().count() > 24 {
                    let skip = lead.chars().count() - 24;
                    format!("…{}", lead.chars().skip(skip).collect::<String>())
                } else {
                    lead.to_string()
                };
                let matched = line[range.clone()].to_string();
                let tail: String = line[range.end..].chars().take(80).collect();
                let replacement = replace.then(|| self.preview_replacement(hit, cx));
                let checked = !self.deselected.contains(hit_ix);
                let toggle_ix = *hit_ix;
                let open = hit.clone();
                base.pl(px(26.))
                    .when(replace, |row| {
                        row.child(
                            Checkbox::new(("search-hit-check", ix))
                                .checked(checked)
                                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                                    this.set_hit_selected(toggle_ix, *checked, cx)
                                })),
                        )
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .text_xs()
                            .child(div().text_color(theme::muted()).child(lead))
                            .child(
                                div()
                                    .bg(theme::accent_wash())
                                    .text_color(theme::text())
                                    .when(replacement.is_some(), |matched| matched.line_through())
                                    .child(matched),
                            )
                            .when_some(replacement, |row, replacement| {
                                row.child(
                                    div()
                                        .bg(gpui::Hsla::from(theme::git_change(
                                            ide_core::git::Change::Added,
                                        ))
                                        .opacity(0.25))
                                        .child(replacement),
                                )
                            })
                            .child(div().text_color(theme::muted()).child(tail)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme::subtle())
                            .child(format!("{}:{}", hit.line, hit.column)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected = ix;
                        cx.emit(SearchPanelEvent::Open(open.clone()));
                        cx.notify();
                    }))
                    .into_any_element()
            }
        }
    }

    fn preview_replacement(&self, hit: &Hit, cx: &App) -> String {
        let options = self.options(cx);
        let replace = self.replacement(cx);
        let Ok(matcher) = project_search::Matcher::new(&options) else {
            return replace;
        };
        // Expand captures against the matched text itself (sufficient for a preview).
        let local: Range<usize> = 0..hit.matched.len();
        matcher
            .replacement(&hit.matched, &local, &replace)
            .unwrap_or(replace)
    }
}

impl Render for SearchPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows = self.rows();
        let row_count = rows.len();
        let has_query = !self.query.read(cx).value().is_empty();
        let selected_count: usize = self
            .selected_hits()
            .iter()
            .map(|(_, hits)| hits.len())
            .sum();
        let scope_label = self.scope.as_ref().map(|scope| {
            self.root
                .as_deref()
                .and_then(|root| scope.strip_prefix(root).ok())
                .map(|relative| relative.display().to_string())
                .filter(|relative| !relative.is_empty())
                .unwrap_or_else(|| scope.display().to_string())
        });
        div()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .debug_selector(|| "search-panel".into())
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::panel())
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if matches!(event.keystroke.key.as_str(), "up" | "down" | "pageup" | "pagedown") {
                    this.key_down(event, window, cx)
                }
            }))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                ui::panel_header("SEARCH")
                    .child(
                        Button::new("search-toggle-replace")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Replace)
                            .tooltip("Toggle Replace")
                            .selected(self.show_replace)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_replace = !this.show_replace;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("search-toggle-filters")
                            .ghost()
                            .xsmall()
                            .icon(IconName::Ellipsis)
                            .tooltip("Files to include/exclude")
                            .selected(self.show_filters)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.show_filters = !this.show_filters;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .px_2()
                    .pb_2()
                    .flex()
                    .flex_col()
                    .gap_1p5()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(div().flex_1().min_w_0().child(Input::new(&self.query).small()))
                            .child(self.toggle("search-case", "Aa", "Match Case", self.case_sensitive, |this, on| this.case_sensitive = on, cx))
                            .child(self.toggle("search-word", "ab", "Match Whole Word", self.whole_word, |this, on| this.whole_word = on, cx))
                            .child(self.toggle("search-regex", ".*", "Use Regular Expression", self.regex, |this, on| this.regex = on, cx)),
                    )
                    .when(self.show_replace, |panel| {
                        panel.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .child(div().flex_1().min_w_0().child(Input::new(&self.replace).small()))
                                .child(
                                    Button::new("search-replace-all")
                                        .xsmall()
                                        .label(format!("Replace {selected_count}"))
                                        .tooltip("Replace the selected results")
                                        .disabled(selected_count == 0)
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(SearchPanelEvent::Replace)
                                        })),
                                ),
                        )
                    })
                    .when(self.show_filters, |panel| {
                        panel
                            .child(Input::new(&self.include).small())
                            .child(Input::new(&self.exclude).small())
                            .child(
                                Checkbox::new("search-ignore-files")
                                    .label("Use .gitignore and default excludes (target, node_modules…)")
                                    .checked(self.use_ignore_files)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.use_ignore_files = *checked;
                                        cx.emit(SearchPanelEvent::Search);
                                    })),
                            )
                    })
                    .when_some(scope_label, |panel, scope| {
                        panel.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_1()
                                .text_xs()
                                .text_color(theme::muted())
                                .child(Icon::new(IconName::Folder).size(px(12.)))
                                .child(div().flex_1().min_w_0().truncate().child(format!("Searching in {scope}")))
                                .child(
                                    Button::new("search-clear-scope")
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::Close)
                                        .tooltip("Search the whole workspace")
                                        .on_click(cx.listener(|this, _, _, cx| this.set_scope(None, cx))),
                                ),
                        )
                    })
                    .child(
                        div()
                            .text_xs()
                            .map(|status| match (&self.error, self.searching, has_query) {
                                (Some(error), _, _) => status
                                    .text_color(theme::git_change(ide_core::git::Change::Deleted))
                                    .child(SharedString::from(error.clone())),
                                (None, true, _) => status.text_color(theme::muted()).child("Searching…"),
                                (None, false, true) => status.text_color(theme::muted()).child(self.summary()),
                                (None, false, false) => status,
                            }),
                    )
                    .when_some(self.report.clone(), |panel, report| {
                        panel.child(div().text_xs().text_color(theme::muted()).child(report))
                    })
                    .when(self.results.truncated, |panel| {
                        panel.child(
                            Button::new("search-load-more")
                                .xsmall()
                                .label("Load more results")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.limit = this.limit.saturating_mul(2);
                                    cx.emit(SearchPanelEvent::LoadMore);
                                })),
                        )
                    })
                    .when(self.show_replace && !self.results.hits.is_empty(), |panel| {
                        panel.child(
                            div()
                                .flex()
                                .gap_1()
                                .child(
                                    Button::new("search-select-all")
                                        .ghost()
                                        .xsmall()
                                        .label("Select all")
                                        .on_click(cx.listener(|this, _, _, cx| this.set_all_selected(true, cx))),
                                )
                                .child(
                                    Button::new("search-select-none")
                                        .ghost()
                                        .xsmall()
                                        .label("Select none")
                                        .on_click(cx.listener(|this, _, _, cx| this.set_all_selected(false, cx))),
                                ),
                        )
                    }),
            )
            .child(
                div().flex_1().min_h_0().child(
                    uniform_list(
                        "search-results",
                        row_count,
                        cx.processor(move |this, range: Range<usize>, _, cx| {
                            let rows = this.rows();
                            range
                                .filter_map(|ix| rows.get(ix).map(|row| this.render_row(row, ix, cx)))
                                .collect::<Vec<_>>()
                        }),
                    )
                    .size_full()
                    .px_1()
                    .track_scroll(self.scroll.clone()),
                ),
            )
    }
}
