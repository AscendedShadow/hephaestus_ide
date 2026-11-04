//! Go to definition/declaration and hover, with a text-search fallback.

use super::*;

impl IdeShell {
    pub(super) fn editor_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.cursors_mouse_down(event, window, cx) {
            return;
        }
        if event.button != MouseButton::Left
            || event.modifiers != Modifiers::shift()
            || self.blocked()
        {
            return;
        }
        let editor = self.editor().read(cx);
        if !editor
            .text_geometry()
            .is_some_and(|geometry| geometry.viewport.contains(&event.position))
        {
            return;
        }
        let offset = editor.index_for_mouse_position(event.position);
        if self.go_to_declaration(offset, window, cx) {
            cx.stop_propagation();
        }
    }

    pub(super) fn go_to_declaration(
        &mut self,
        display_offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let buffer = self.buffer();
        let language = buffer.language;
        let offset = folding::display_to_source(&buffer.folds, display_offset, false);
        let position = position_at(&buffer.document.text().to_string(), offset);
        let request = buffer
            .document
            .path()
            .and_then(|path| self.servers.read(cx).definition(path, language, position));
        let Some(request) = request else {
            return self.go_to_declaration_fallback(display_offset, window, cx);
        };
        self.status = "Looking up the declaration…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let found = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let shown = found
                    .is_some_and(|response| this.show_definition_targets(response, window, cx));
                if !shown {
                    this.go_to_declaration_fallback(display_offset, window, cx);
                }
            });
        });
        true
    }

    pub(super) fn go_to_definition_action(
        &mut self,
        _: &GoToDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.show_diff {
            return;
        }
        let offset = self.editor().read(cx).cursor();
        self.go_to_declaration(offset, window, cx);
    }

    pub(super) fn hover_info(
        &mut self,
        _: &HoverInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = self.buffer();
        let editor = buffer.editor.clone();
        let cursor = editor.read(cx).cursor();
        let offset = folding::display_to_source(&buffer.folds, cursor, false);
        let source = buffer.document.text().to_string();
        let language = buffer.language;
        let origin = buffer.document.path().map(Path::to_path_buf);
        let request = origin.as_deref().and_then(|path| {
            self.servers
                .read(cx)
                .hover(path, language, position_at(&source, offset))
        });
        let root = self.project_directory().map(Path::to_path_buf);
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let mut hover = match request {
                Some(request) => request.await,
                None => None,
            };
            if hover.is_none() {
                let source = source.clone();
                hover = cx
                    .background_executor()
                    .spawn(async move {
                        language_server::declaration_hover(
                            language,
                            &source,
                            offset,
                            root.as_deref(),
                            origin.as_deref(),
                        )
                    })
                    .await;
            }
            let _ = this.update_in(cx, |this, window, cx| match hover {
                Some(hover) => {
                    editor.update(cx, |editor, cx| {
                        editor.show_hover(cursor, &hover, window, cx)
                    });
                    this.status = "Showing the declaration".into();
                }
                None => this.status = "No declaration information here".into(),
            });
        });
        self.status = "Looking up the declaration…".into();
        cx.notify();
    }

    pub(super) fn go_to_declaration_fallback(
        &mut self,
        display_offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let buffer = self.buffer();
        let source = buffer.document.text().to_string();
        let offset = folding::display_to_source(&buffer.folds, display_offset, false);
        let language = buffer.language;
        let Some(lookup) = navigation::lookup(language, &source, offset) else {
            return false;
        };
        self.declaration_search = Task::ready(());
        match lookup {
            Lookup::Found(name) => {
                self.reveal_declaration(self.active, name, window, cx);
                true
            }
            Lookup::Search(search) => self.search_declaration(search, language, window, cx),
        }
    }

    pub(super) fn search_declaration(
        &mut self,
        search: Search,
        language: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(root) = self.project_directory().map(Path::to_path_buf) else {
            let Some(fallback) = search.fallback else {
                return false;
            };
            self.reveal_declaration(self.active, fallback, window, cx);
            return true;
        };
        let origin = self.buffer().id;
        let path = self.document().path().map(Path::to_path_buf);
        let open: HashMap<PathBuf, String> = self
            .buffers
            .iter()
            .filter_map(|buffer| {
                let path = buffer.document.path()?.to_path_buf();
                Some((path, buffer.document.text().to_string()))
            })
            .collect();
        self.status = format!("Looking for the declaration of {}…", search.name);
        cx.notify();
        let executor = cx.background_executor().clone();
        self.declaration_search = cx.spawn_in(window, async move |this, cx| {
            let hit = executor
                .spawn({
                    let search = search.clone();
                    async move {
                        navigation::search_workspace(
                            &root,
                            path.as_deref(),
                            language,
                            &search,
                            &open,
                        )
                    }
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.open_declaration(origin, search, hit, window, cx)
            });
        });
        true
    }

    pub(super) fn open_declaration(
        &mut self,
        origin: BufferId,
        search: Search,
        hit: Option<Hit>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.blocked() {
            return;
        }
        let Search { name, fallback, .. } = search;
        let target = hit
            .and_then(|hit| {
                match self.buffer_for_path(&hit.path) {
                    Some(ix) => self.activate(ix, window, cx),
                    None => self.open_document(hit.document?, window, cx),
                }
                Some((self.active, hit.name))
            })
            .or_else(|| Some((self.buffer_index(origin)?, fallback?)));
        match target {
            Some((ix, declaration)) => {
                if ix != self.active {
                    self.activate(ix, window, cx);
                }
                self.reveal_declaration(ix, declaration, window, cx);
            }
            None => {
                self.status = format!("No declaration found for {name}");
                cx.notify();
            }
        }
    }

    pub(super) fn reveal_declaration(
        &mut self,
        ix: usize,
        declaration: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let buffer = &mut self.buffers[ix];
        let source = buffer.document.text().to_string();
        let offset = declaration.start.min(source.len());
        let editor = buffer.editor.clone();
        let display = match folding::reveal(&source, &mut buffer.folds, offset) {
            Some(toggle) => {
                editor.update(cx, |editor, cx| {
                    let text = editor.text();
                    let range = text.byte_to_utf16_idx(toggle.edit.start)
                        ..text.byte_to_utf16_idx(toggle.edit.end);
                    editor.replace_text_in_range(Some(range), &toggle.text, window, cx);
                });
                toggle.cursor
            }
            None => folding::source_to_display(&buffer.folds, offset),
        };
        let name = source.get(declaration).unwrap_or_default();
        let line = position_at(&source, offset).line + 1;
        self.status = format!("Declaration of {name} — {}:{line}", buffer.document.name());
        editor.update(cx, |editor, cx| editor.reveal(display, window, cx));
        cx.notify();
    }
}
