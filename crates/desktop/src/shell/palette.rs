//! The search palette: Quick Open, commands, tasks, go to line and file search.

use super::*;

impl IdeShell {
    pub(super) fn show_search(
        &mut self,
        mode: SearchMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_mode = Some(mode);
        self.search_selected = 0;
        self.search_hits.clear();
        self.search_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
            input.focus(window, cx);
        });
        cx.notify();
    }

    pub(super) fn quick_open(
        &mut self,
        _: &QuickOpen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search(SearchMode::Files, window, cx);
    }

    pub(super) fn command_palette(
        &mut self,
        _: &CommandPalette,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search(SearchMode::Commands, window, cx);
    }

    pub(super) fn show_diagnostics(
        &mut self,
        _: &ShowDiagnostics,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search(SearchMode::Diagnostics, window, cx);
    }

    pub(super) fn find_in_file(
        &mut self,
        _: &FindInFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_search(SearchMode::File, window, cx);
    }

    pub(super) fn go_to_line(&mut self, _: &GoToLine, window: &mut Window, cx: &mut Context<Self>) {
        self.show_search(SearchMode::Line, window, cx);
    }

    pub(super) fn update_search(&mut self, query: String, cx: &mut Context<Self>) {
        self.search_task = Task::ready(());
        self.search_hits.clear();
        self.search_selected = 0;
        if self.search_mode == Some(SearchMode::Items)
            && self
                .picker
                .as_ref()
                .is_some_and(|picker| picker.kind == picker::PickerKind::WorkspaceSymbols)
        {
            self.update_workspace_symbols(query.clone(), cx);
        }
        match self.search_mode {
            Some(SearchMode::File) if !query.is_empty() => {
                let text = self.document().text().to_string();
                let path = self
                    .document()
                    .path()
                    .unwrap_or(Path::new("Untitled"))
                    .to_path_buf();
                self.search_hits = search::search_source(&path, &text, &query, 200);
            }
            _ => {}
        }
        if self.search_mode == Some(SearchMode::Diagnostics) {
            self.search_hits = self
                .diagnostics
                .values()
                .flatten()
                .chain(self.run_hits.iter())
                .filter(|hit| {
                    query.is_empty() || hit.preview.to_lowercase().contains(&query.to_lowercase())
                })
                .cloned()
                .collect();
            self.search_hits
                .sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.line.cmp(&b.line)));
        }
        cx.notify();
    }

    pub(super) fn choose_search_result(
        &mut self,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(mode) = self.search_mode else { return };
        let query = self.search_input.read(cx).text().to_string();
        let query = query.trim_end_matches(['\r', '\n']).to_string();
        let query = query.as_str();
        let file = (mode == SearchMode::Files)
            .then(|| self.ranked_paths(query).get(ix).cloned())
            .flatten();
        let command = (mode == SearchMode::Commands)
            .then(|| self.ranked_commands(query).get(ix).copied())
            .flatten();
        let task = (mode == SearchMode::Tasks)
            .then(|| self.ranked_tasks(query).get(ix).copied())
            .flatten();
        let language = (mode == SearchMode::Languages)
            .then(|| {
                self.ranked_languages(query)
                    .get(ix)
                    .map(|language| language.id)
            })
            .flatten();
        let hit = self.search_hits.get(ix).cloned();
        self.search_mode = None;
        match mode {
            SearchMode::Files => {
                if let Some(path) = file {
                    self.open(Some(path), window, cx);
                }
            }
            SearchMode::Commands => {
                if let Some(command) = command {
                    window.dispatch_action(command.action(), cx);
                }
            }
            SearchMode::Tasks => {
                if let Some(index) = task {
                    self.launch_config(index, window, cx);
                }
            }
            SearchMode::Items => self.choose_picker_item(ix, query, window, cx),
            SearchMode::Languages => {
                if let Some(language) = language {
                    self.set_buffer_language(self.active, language, cx);
                }
                self.focus_editor(window, cx);
            }
            SearchMode::Line => {
                let parts: Vec<_> = query.split(':').collect();
                if let Some(line) = parts
                    .first()
                    .and_then(|value| value.parse::<usize>().ok())
                    .filter(|line| *line > 0)
                {
                    let column = parts
                        .get(1)
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(1)
                        .max(1);
                    let path = self
                        .document()
                        .path()
                        .unwrap_or(Path::new("Untitled"))
                        .to_path_buf();
                    let hit = SearchMatch {
                        path,
                        line,
                        column,
                        preview: String::new(),
                    };
                    self.record_location(&hit, cx);
                    self.jump_to_hit(&hit, window, cx);
                }
            }
            SearchMode::File => {
                if let Some(hit) = hit {
                    self.record_location(&hit, cx);
                    self.jump_to_hit(&hit, window, cx);
                }
            }
            SearchMode::Diagnostics => {
                if let Some(hit) = hit {
                    self.record_location(&hit, cx);
                    if self.document().path() == Some(hit.path.as_path()) {
                        self.jump_to_hit(&hit, window, cx);
                    } else {
                        self.open_at(hit, window, cx);
                    }
                }
            }
        }
        cx.notify();
    }

    /// Quick Open candidates for `query`, best first (at most 30). Memoized per query,
    /// since render and key handling ask for the same list repeatedly.
    pub(super) fn ranked_paths(&self, query: &str) -> Vec<PathBuf> {
        let key = (
            query.to_string(),
            self.search_paths_generation.get(),
            self.recent_paths.first().cloned(),
        );
        if let Some((cached, paths)) = &*self.ranked_paths_cache.borrow()
            && *cached == key
        {
            return paths.clone();
        }
        let paths = self.rank_paths(query);
        *self.ranked_paths_cache.borrow_mut() = Some((key, paths.clone()));
        paths
    }

    pub(super) fn rank_paths(&self, query: &str) -> Vec<PathBuf> {
        const LIMIT: usize = 30;
        let query = query.to_lowercase();
        let qualifier = self.workspace.qualifier();
        let recent: HashMap<&Path, usize> = self
            .recent_paths
            .iter()
            .enumerate()
            .map(|(ix, path)| (path.as_path(), ix))
            .collect();
        let mut files: Vec<(usize, usize, &PathBuf)> = self
            .search_paths
            .iter()
            .filter_map(|path| {
                let name = qualifier.qualify(path).to_lowercase();
                search::rank_lowercase(&name, &query).map(|score| {
                    let recency = recent.get(path.as_path()).copied().unwrap_or(usize::MAX);
                    (score, recency, path)
                })
            })
            .collect();
        // Only the best LIMIT entries are shown: select them, then sort just those.
        if files.len() > LIMIT {
            files.select_nth_unstable(LIMIT);
            files.truncate(LIMIT);
        }
        files.sort();
        files.into_iter().map(|(_, _, path)| path.clone()).collect()
    }

    /// Note that `search_paths` changed.
    pub(super) fn bump_search_paths(&self) {
        self.search_paths_generation
            .set(self.search_paths_generation.get() + 1);
    }

    pub(super) fn ranked_commands(&self, query: &str) -> Vec<Command> {
        Command::ALL
            .iter()
            .copied()
            .filter(|command| search::rank(Path::new(&command.name()), query).is_some())
            .collect()
    }

    pub(super) fn ranked_tasks(&self, query: &str) -> Vec<usize> {
        let mut ranked: Vec<_> = self
            .task_commands
            .iter()
            .enumerate()
            .filter_map(|(ix, config)| {
                search::rank(Path::new(&config.name), query).map(|score| (score, ix))
            })
            .collect();
        ranked.sort();
        ranked.into_iter().take(30).map(|(_, ix)| ix).collect()
    }

    pub(super) fn search_item_count(&self, cx: &App) -> usize {
        let query = self.search_input.read(cx).text().to_string();
        let query = query.trim_end_matches(['\r', '\n']);
        match self.search_mode {
            Some(SearchMode::Files) => self.ranked_paths(query).len(),
            Some(SearchMode::Commands) => self.ranked_commands(query).len(),
            Some(SearchMode::Tasks) => self.ranked_tasks(query).len(),
            Some(SearchMode::Languages) => self.ranked_languages(query).len(),
            Some(SearchMode::Items) => self.ranked_items(query).len(),
            Some(SearchMode::File | SearchMode::Diagnostics) => self.search_hits.len().min(1000),
            Some(SearchMode::Line) | None => 1,
        }
    }

    pub(super) fn render_search(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.search_mode.unwrap();
        let query = self.search_input.read(cx).text().to_string();
        let query = query.trim_end_matches(['\r', '\n']);
        let items: Vec<String> = match mode {
            SearchMode::Files => self
                .ranked_paths(query)
                .iter()
                .map(|path| self.workspace.qualified(path))
                .collect(),
            SearchMode::Commands => self
                .ranked_commands(query)
                .iter()
                .map(|command| {
                    format!(
                        "{}  {}",
                        command.name(),
                        shortcut(command.action().as_ref(), cx).unwrap_or_default()
                    )
                })
                .collect(),
            SearchMode::Tasks => self
                .ranked_tasks(query)
                .iter()
                .map(|&ix| {
                    let config = &self.task_commands[ix];
                    format!("{}  {}", config.name, config.program)
                })
                .collect(),
            SearchMode::Line => vec!["Enter line or line:column, then press Enter".into()],
            SearchMode::Items => self.picker_labels(query),
            SearchMode::Languages => self
                .ranked_languages(query)
                .into_iter()
                .map(Self::language_label)
                .collect(),
            SearchMode::File | SearchMode::Diagnostics => self
                .search_hits
                .iter()
                .take(1000)
                .map(|hit| {
                    format!(
                        "{}:{}:{}  {}",
                        hit.path.display(),
                        hit.line,
                        hit.column,
                        hit.preview
                    )
                })
                .collect(),
        };
        div()
            .p_2()
            .bg(theme::panel())
            .border_b_1()
            .border_color(theme::border())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                match event.keystroke.key.as_str() {
                    "escape" => {
                        this.search_mode = None;
                        this.focus_editor(window, cx);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "up" => {
                        this.search_selected = this.search_selected.saturating_sub(1);
                        cx.stop_propagation();
                        cx.notify();
                    }
                    "down" => {
                        this.search_selected = (this.search_selected + 1)
                            .min(this.search_item_count(cx).saturating_sub(1));
                        cx.stop_propagation();
                        cx.notify();
                    }
                    _ => {}
                }
            }))
            .child(Input::new(&self.search_input))
            .child(div().max_h(px(260.)).overflow_y_scrollbar().children(
                items.into_iter().enumerate().map(|(ix, label)| {
                    div()
                        .id(("search-hit", ix))
                        .px_2()
                        .py_1()
                        .cursor_pointer()
                        .when(ix == self.search_selected, |row| row.bg(theme::hover()))
                        .hover(|style| style.bg(theme::hover()))
                        .child(label)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.choose_search_result(ix, window, cx)
                        }))
                }),
            ))
    }
}
