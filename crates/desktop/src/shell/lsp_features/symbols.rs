//! Document and workspace symbols.

use super::*;

impl IdeShell {
    /// Request document symbols for the active buffer; `then` receives the flattened
    /// symbols if the response is still current.
    pub(in crate::shell) fn request_document_symbols(
        &mut self,
        quiet: bool,
        then: fn(&mut Self, Vec<Symbol>, &mut Window, &mut Context<Self>),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let target = if quiet {
            let ix = self.active;
            let Some(path) = self.buffers[ix].document.path().map(Path::to_path_buf) else {
                return then(self, Vec::new(), window, cx);
            };
            let servers = self.servers.read(cx);
            Target {
                ix,
                id: self.buffers[ix].id,
                language: self.buffers[ix].language,
                position: lsp_types::Position::default(),
                origin: servers.origin(&path),
                features: servers.features(self.buffers[ix].language),
                path,
            }
        } else {
            match self.lsp_target(cx) {
                Some(target) => target,
                None => return,
            }
        };
        if !target.features.document_symbols {
            if !quiet {
                self.unsupported("Go to Symbol", &target, cx);
            }
            return then(self, Vec::new(), window, cx);
        }
        let Some(document) = Servers::text_document(&target.path) else {
            return;
        };
        let Some(request) = self.servers.read(cx).send::<DocumentSymbolRequest>(
            target.language,
            DocumentSymbolParams {
                text_document: document,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        ) else {
            return;
        };
        self.symbol_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                // Results belong to the originating document and version only.
                if !this.servers.read(cx).is_current(&target.origin) {
                    return;
                }
                let Some(ix) = this.buffer_index(target.id) else {
                    return;
                };
                let text = this.buffers[ix].document.text().to_string();
                let symbols = match response {
                    Ok(Some(response)) => flatten_symbols(response, &text),
                    Ok(None) => Vec::new(),
                    Err(error) => {
                        if !quiet {
                            this.status = format!("Document symbols failed: {error}");
                        }
                        Vec::new()
                    }
                };
                this.outline = Some((target.id, target.origin.version, symbols.clone()));
                then(this, symbols, window, cx);
                cx.notify();
            });
        });
    }

    pub(in crate::shell) fn go_to_symbol_in_file(
        &mut self,
        _: &GoToSymbolInFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_document_symbols(
            false,
            |this, symbols, window, cx| {
                let Some(path) = this.document().path().map(Path::to_path_buf) else {
                    return;
                };
                let text = this.document().text().to_string();
                let items = symbols
                    .iter()
                    .map(|symbol| {
                        let (line, character) =
                            ide_core::text_edit::position(&text, symbol.selection.start);
                        PickerItem {
                            label: format!("{}  {}", symbol.name, kind_label(symbol.kind)),
                            detail: symbol.detail.clone(),
                            filter: symbol.name.clone(),
                            depth: symbol.depth,
                            action: PickerAction::Open(SearchMatch {
                                path: path.clone(),
                                line: line as usize + 1,
                                column: character as usize + 1,
                                preview: String::new(),
                            }),
                        }
                    })
                    .collect();
                let title = format!("{} symbols", symbols.len());
                this.show_picker(PickerKind::DocumentSymbols, title, items, window, cx);
            },
            window,
            cx,
        );
    }

    pub(in crate::shell) fn go_to_symbol_in_workspace(
        &mut self,
        _: &GoToSymbolInWorkspace,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.workspace_symbols {
            return self.unsupported("Go to Symbol in Workspace", &target, cx);
        }
        self.workspace_symbols = Vec::new();
        self.workspace_symbol_language = Some(target.language);
        self.show_picker(
            PickerKind::WorkspaceSymbols,
            "Type to search workspace symbols".into(),
            Vec::new(),
            window,
            cx,
        );
    }

    /// Query workspace symbols as the picker's query changes.
    pub(in crate::shell) fn update_workspace_symbols(
        &mut self,
        query: String,
        cx: &mut Context<Self>,
    ) {
        let Some(language) = self.workspace_symbol_language else {
            return;
        };
        let Some(request) = self.servers.read(cx).send::<WorkspaceSymbolRequest>(
            language,
            WorkspaceSymbolParams {
                query: query.clone(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        ) else {
            return;
        };
        let root = self.workspace.root().map(Path::to_path_buf);
        self.symbol_task = cx.spawn(async move |this, cx| {
            let response = request.await;
            let _ = this.update(cx, |this, cx| {
                if this.search_mode != Some(SearchMode::Items)
                    || *this.search_input.read(cx).text() != query.as_str()
                {
                    return;
                }
                let symbols: Vec<lsp_types::WorkspaceSymbol> = match response {
                    Ok(Some(WorkspaceSymbolResponse::Flat(list))) => list
                        .into_iter()
                        .map(|symbol| lsp_types::WorkspaceSymbol {
                            name: symbol.name,
                            kind: symbol.kind,
                            tags: symbol.tags,
                            container_name: symbol.container_name,
                            location: lsp_types::OneOf::Left(symbol.location),
                            data: None,
                        })
                        .collect(),
                    Ok(Some(WorkspaceSymbolResponse::Nested(list))) => list,
                    _ => Vec::new(),
                };
                let items = symbols
                    .iter()
                    .enumerate()
                    .map(|(ix, symbol)| {
                        let uri = match &symbol.location {
                            lsp_types::OneOf::Left(location) => &location.uri,
                            lsp_types::OneOf::Right(location) => &location.uri,
                        };
                        let path = lsp::uri_to_path(uri).unwrap_or_default();
                        let relative = root
                            .as_deref()
                            .and_then(|root| path.strip_prefix(root).ok())
                            .unwrap_or(&path)
                            .display()
                            .to_string();
                        let container = symbol
                            .container_name
                            .as_deref()
                            .map(|name| format!("{name} · "))
                            .unwrap_or_default();
                        PickerItem {
                            label: format!("{}  {}", symbol.name, kind_label(symbol.kind)),
                            detail: format!("{container}{relative}"),
                            filter: symbol.name.clone(),
                            depth: 0,
                            action: PickerAction::WorkspaceSymbol(ix),
                        }
                    })
                    .collect();
                this.workspace_symbols = symbols;
                if let Some(picker) = &mut this.picker {
                    picker.title = format!("{} workspace symbols", this.workspace_symbols.len());
                    picker.items = items;
                }
                cx.notify();
            });
        });
    }

    pub(in crate::shell) fn open_workspace_symbol(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(symbol) = self.workspace_symbols.get(index).cloned() else {
            return;
        };
        let Some(language) = self.workspace_symbol_language else {
            return;
        };
        match symbol.location.clone() {
            lsp_types::OneOf::Left(location) => {
                if let Some(path) = lsp::uri_to_path(&location.uri) {
                    self.show_locations("symbols", vec![(path, location.range)], false, window, cx);
                }
            }
            lsp_types::OneOf::Right(_) => {
                // The server returned a location without a range: resolve it.
                let Some(request) = self
                    .servers
                    .read(cx)
                    .send::<WorkspaceSymbolResolve>(language, symbol)
                else {
                    return;
                };
                self.lsp_task = cx.spawn_in(window, async move |this, cx| {
                    let resolved = request.await;
                    let _ = this.update_in(cx, |this, window, cx| match resolved {
                        Ok(symbol) => match symbol.location {
                            lsp_types::OneOf::Left(location) => {
                                if let Some(path) = lsp::uri_to_path(&location.uri) {
                                    this.show_locations(
                                        "symbols",
                                        vec![(path, location.range)],
                                        false,
                                        window,
                                        cx,
                                    );
                                }
                            }
                            lsp_types::OneOf::Right(location) => {
                                if let Some(path) = lsp::uri_to_path(&location.uri) {
                                    this.open(Some(path), window, cx);
                                }
                            }
                        },
                        Err(error) => {
                            this.status = format!("Could not resolve the symbol: {error}");
                            cx.notify();
                        }
                    });
                });
            }
        }
    }
}
