//! Language-server commands: rename (IDE-005), code actions (IDE-006), formatting
//! (IDE-007), references and implementation/type navigation (IDE-008) and symbols
//! (IDE-009).
//!
//! Every request records the document version it was made for; a response that arrives
//! after the document changed (or for a buffer that is no longer active, where focus
//! would move unexpectedly) is discarded with an explanation instead of being applied.

use std::collections::HashMap;

use gpui_component::input::InputState;
use lsp_types::{
    CodeActionContext, CodeActionOrCommand, CodeActionParams, DocumentFormattingParams,
    DocumentRangeFormattingParams, DocumentSymbolParams, DocumentSymbolResponse, FormattingOptions,
    GotoDefinitionParams, GotoDefinitionResponse, Location, LocationLink, PrepareRenameResponse,
    ReferenceContext, ReferenceParams, RenameParams, SymbolKind, WorkspaceEdit,
    WorkspaceSymbolParams, WorkspaceSymbolResponse,
    request::{
        CodeActionRequest, CodeActionResolveRequest, DocumentSymbolRequest, ExecuteCommand,
        Formatting, GotoImplementation, GotoTypeDefinition, PrepareRenameRequest, RangeFormatting,
        References, Rename, WorkspaceSymbolRequest, WorkspaceSymbolResolve,
    },
};

use super::picker::{PickerAction, PickerItem, PickerKind};
use super::*;
use crate::language_server::{Features, RequestOrigin, Servers};

type FormattingFuture =
    std::pin::Pin<Box<dyn Future<Output = Result<Option<Vec<lsp_types::TextEdit>>, lsp::Error>>>>;

/// The active buffer's language-server context at the cursor.
pub(super) struct Target {
    pub ix: usize,
    pub id: BufferId,
    pub path: PathBuf,
    pub language: &'static str,
    pub position: lsp_types::Position,
    pub origin: RequestOrigin,
    pub features: Features,
}

/// A flattened document symbol.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Symbol {
    pub name: String,
    pub detail: String,
    pub kind: SymbolKind,
    pub depth: usize,
    /// Source byte ranges (whole symbol, and the name to reveal).
    pub range: Range<usize>,
    pub selection: Range<usize>,
}

pub(super) fn kind_label(kind: SymbolKind) -> &'static str {
    match kind {
        SymbolKind::FILE => "file",
        SymbolKind::MODULE => "module",
        SymbolKind::NAMESPACE => "namespace",
        SymbolKind::PACKAGE => "package",
        SymbolKind::CLASS => "class",
        SymbolKind::METHOD => "method",
        SymbolKind::PROPERTY => "property",
        SymbolKind::FIELD => "field",
        SymbolKind::CONSTRUCTOR => "constructor",
        SymbolKind::ENUM => "enum",
        SymbolKind::INTERFACE => "interface",
        SymbolKind::FUNCTION => "function",
        SymbolKind::VARIABLE => "variable",
        SymbolKind::CONSTANT => "constant",
        SymbolKind::STRUCT => "struct",
        SymbolKind::ENUM_MEMBER => "variant",
        SymbolKind::TYPE_PARAMETER => "type parameter",
        SymbolKind::EVENT => "event",
        SymbolKind::OPERATOR => "operator",
        SymbolKind::OBJECT => "object",
        _ => "symbol",
    }
}

/// Flatten hierarchical or flat symbol responses into depth-annotated source ranges.
pub(super) fn flatten_symbols(response: DocumentSymbolResponse, text: &str) -> Vec<Symbol> {
    let range = |range: &lsp_types::Range| {
        let start = ide_core::text_edit::offset(text, range.start.line, range.start.character)
            .unwrap_or(text.len());
        let end = ide_core::text_edit::offset(text, range.end.line, range.end.character)
            .unwrap_or(text.len());
        start..end.max(start)
    };
    let mut symbols = Vec::new();
    match response {
        DocumentSymbolResponse::Nested(nested) => {
            fn walk(
                list: &[lsp_types::DocumentSymbol],
                depth: usize,
                range: &dyn Fn(&lsp_types::Range) -> Range<usize>,
                out: &mut Vec<Symbol>,
            ) {
                let mut sorted: Vec<_> = list.iter().collect();
                sorted
                    .sort_by_key(|symbol| (symbol.range.start.line, symbol.range.start.character));
                for symbol in sorted {
                    out.push(Symbol {
                        name: symbol.name.clone(),
                        detail: symbol.detail.clone().unwrap_or_default(),
                        kind: symbol.kind,
                        depth,
                        range: range(&symbol.range),
                        selection: range(&symbol.selection_range),
                    });
                    if let Some(children) = &symbol.children {
                        walk(children, depth + 1, range, out);
                    }
                }
            }
            walk(&nested, 0, &range, &mut symbols);
        }
        DocumentSymbolResponse::Flat(flat) => {
            // Rebuild nesting from containment of ranges.
            let mut items: Vec<_> = flat
                .iter()
                .map(|symbol| (range(&symbol.location.range), symbol))
                .collect();
            items.sort_by(|a, b| a.0.start.cmp(&b.0.start).then(b.0.end.cmp(&a.0.end)));
            let mut stack: Vec<Range<usize>> = Vec::new();
            for (symbol_range, symbol) in items {
                while stack.last().is_some_and(|parent| {
                    !(parent.start <= symbol_range.start && symbol_range.end <= parent.end)
                }) {
                    stack.pop();
                }
                symbols.push(Symbol {
                    name: symbol.name.clone(),
                    detail: symbol.container_name.clone().unwrap_or_default(),
                    kind: symbol.kind,
                    depth: stack.len(),
                    range: symbol_range.clone(),
                    selection: symbol_range.clone(),
                });
                stack.push(symbol_range);
            }
        }
    }
    symbols
}

/// Normalize definition-style responses to `(path, range)` targets.
pub(super) fn locations(response: GotoDefinitionResponse) -> Vec<(PathBuf, lsp_types::Range)> {
    let from_location =
        |location: Location| Some((lsp::uri_to_path(&location.uri)?, location.range));
    let from_link = |link: LocationLink| {
        Some((
            lsp::uri_to_path(&link.target_uri)?,
            link.target_selection_range,
        ))
    };
    let mut targets: Vec<_> = match response {
        GotoDefinitionResponse::Scalar(location) => from_location(location).into_iter().collect(),
        GotoDefinitionResponse::Array(list) => list.into_iter().filter_map(from_location).collect(),
        GotoDefinitionResponse::Link(list) => list.into_iter().filter_map(from_link).collect(),
    };
    targets.dedup();
    targets
}

impl IdeShell {
    /// The LSP context of the active buffer, or a status message explaining why there is
    /// none.
    pub(super) fn lsp_target(&mut self, cx: &mut Context<Self>) -> Option<Target> {
        if self.show_diff || self.blocked() {
            return None;
        }
        let ix = self.active;
        let buffer = &self.buffers[ix];
        let Some(path) = buffer.document.path().map(Path::to_path_buf) else {
            self.status = "Save the file to use language features".into();
            cx.notify();
            return None;
        };
        let servers = self.servers.read(cx);
        if servers.server_capabilities(buffer.language).is_none() {
            self.status = format!(
                "No language server is running for {}",
                ide_core::language::get(buffer.language).name
            );
            cx.notify();
            return None;
        }
        let (selection, _) = self.source_selection(ix, cx);
        let text = buffer.document.text().to_string();
        let cursor = self.buffers[ix].editor.read(cx).cursor();
        let cursor = folding::display_to_source(&self.buffers[ix].folds, cursor, false);
        let _ = selection;
        let (line, character) = ide_core::text_edit::position(&text, cursor);
        Some(Target {
            ix,
            id: self.buffers[ix].id,
            language: self.buffers[ix].language,
            position: lsp_types::Position::new(line, character),
            origin: servers.origin(&path),
            features: servers.features(self.buffers[ix].language),
            path,
        })
    }

    fn unsupported(&mut self, what: &str, target: &Target, cx: &mut Context<Self>) {
        self.status = format!(
            "{what} is not supported by the {} language server",
            ide_core::language::get(target.language).name
        );
        cx.notify();
    }

    /// Whether a response for `target` can still be applied.
    fn still_current(&mut self, target: &Target, what: &str, cx: &mut Context<Self>) -> bool {
        let current = self.servers.read(cx).is_current(&target.origin)
            && self.buffer_index(target.id).is_some();
        if !current {
            self.status = format!("{what} result discarded: the document changed");
            cx.notify();
        }
        current
    }

    fn source_range(&self, ix: usize, cx: &App) -> lsp_types::Range {
        let (selection, _) = self.source_selection(ix, cx);
        let text = self.buffers[ix].document.text().to_string();
        let (start_line, start_character) = ide_core::text_edit::position(&text, selection.start);
        let (end_line, end_character) = ide_core::text_edit::position(&text, selection.end);
        lsp_types::Range::new(
            lsp_types::Position::new(start_line, start_character),
            lsp_types::Position::new(end_line, end_character),
        )
    }

    fn location_item(&self, path: &Path, range: lsp_types::Range, cx: &App) -> PickerItem {
        let text = match self.buffer_for_path(path) {
            Some(ix) => self.buffers[ix].document.text().to_string(),
            None => Document::open(path)
                .map(|document| document.text().to_string())
                .unwrap_or_default(),
        };
        let _ = cx;
        let line_text = text
            .lines()
            .nth(range.start.line as usize)
            .unwrap_or_default()
            .trim()
            .chars()
            .take(120)
            .collect::<String>();
        let start = ide_core::text_edit::offset(&text, range.start.line, range.start.character)
            .unwrap_or(0);
        let line_start = text[..start.min(text.len())]
            .rfind('\n')
            .map_or(0, |ix| ix + 1);
        let column = text[line_start..start.min(text.len())]
            .encode_utf16()
            .count()
            + 1;
        let relative = self
            .workspace
            .root()
            .and_then(|root| path.strip_prefix(root).ok())
            .unwrap_or(path)
            .display()
            .to_string();
        let location = SearchMatch {
            path: path.to_path_buf(),
            line: range.start.line as usize + 1,
            column,
            preview: line_text.clone(),
        };
        PickerItem {
            label: format!("{relative}:{}:{column}", location.line),
            detail: line_text,
            filter: relative,
            depth: 0,
            action: PickerAction::Open(location),
        }
    }

    /// Show navigation targets: jump directly to a single target, offer a chooser for
    /// several, explain an empty result.
    fn show_locations(
        &mut self,
        what: &str,
        mut targets: Vec<(PathBuf, lsp_types::Range)>,
        grouped: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if targets.is_empty() {
            self.status = format!("No {what} found");
            cx.notify();
            return;
        }
        targets.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then(a.1.start.line.cmp(&b.1.start.line))
                .then(a.1.start.character.cmp(&b.1.start.character))
        });
        targets.dedup();
        if targets.len() == 1 && !grouped {
            let item = self.location_item(&targets[0].0, targets[0].1, cx);
            if let PickerAction::Open(location) = item.action {
                self.record_location(&location, cx);
                self.open_at(location, window, cx);
            }
            return;
        }
        let mut items = Vec::new();
        let files = {
            let mut paths: Vec<_> = targets.iter().map(|(path, _)| path.clone()).collect();
            paths.dedup();
            paths.len()
        };
        let mut last: Option<PathBuf> = None;
        for (path, range) in &targets {
            if grouped && last.as_ref() != Some(path) {
                let count = targets.iter().filter(|(other, _)| other == path).count();
                let relative = self
                    .workspace
                    .root()
                    .and_then(|root| path.strip_prefix(root).ok())
                    .unwrap_or(path)
                    .display()
                    .to_string();
                items.push(PickerItem {
                    label: format!("{relative} ({count})"),
                    detail: String::new(),
                    filter: relative,
                    depth: 0,
                    action: PickerAction::Nothing,
                });
                last = Some(path.clone());
            }
            let mut item = self.location_item(path, *range, cx);
            if grouped {
                item.depth = 1;
                item.label = format!("{}:{}", range.start.line + 1, range.start.character + 1);
            }
            items.push(item);
        }
        let title = format!("{} {what} in {files} file(s)", targets.len());
        self.status = title.clone();
        self.show_picker(PickerKind::Locations, title, items, window, cx);
    }

    // ----- IDE-008: references, implementations, type definitions -----

    fn definition_like<R>(
        &mut self,
        what: &'static str,
        supported: fn(&Features) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) where
        R: 'static
            + lsp_types::request::Request<
                Params = GotoDefinitionParams,
                Result = Option<GotoDefinitionResponse>,
            >,
    {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !supported(&target.features) {
            return self.unsupported(what, &target, cx);
        }
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let request = self.servers.read(cx).send::<R>(
            target.language,
            GotoDefinitionParams {
                text_document_position_params: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        );
        let Some(request) = request else { return };
        self.status = format!("Looking up {what}…");
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, what, cx) || this.buffer().id != target.id {
                    return;
                }
                match response {
                    Ok(Some(response)) => {
                        this.show_locations(what, locations(response), false, window, cx)
                    }
                    Ok(None) => {
                        this.status = format!("No {what} found");
                        cx.notify();
                    }
                    Err(error) => {
                        this.status = format!("Could not find {what}: {error}");
                        cx.notify();
                    }
                }
            });
        });
    }

    pub(super) fn go_to_implementation(
        &mut self,
        _: &GoToImplementation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.definition_like::<GotoImplementation>(
            "implementations",
            |f| f.implementation,
            window,
            cx,
        );
    }

    pub(super) fn go_to_type_definition(
        &mut self,
        _: &GoToTypeDefinition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.definition_like::<GotoTypeDefinition>(
            "type definitions",
            |f| f.type_definition,
            window,
            cx,
        );
    }

    fn references(
        &mut self,
        include_declaration: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.references {
            return self.unsupported("Find References", &target, cx);
        }
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let request = self.servers.read(cx).send::<References>(
            target.language,
            ReferenceParams {
                text_document_position: position,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: ReferenceContext {
                    include_declaration,
                },
            },
        );
        let Some(request) = request else { return };
        self.status = "Finding references…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, "Find References", cx)
                    || this.buffer().id != target.id
                {
                    return;
                }
                match response {
                    Ok(list) => {
                        let targets = list
                            .unwrap_or_default()
                            .into_iter()
                            .filter_map(|location| {
                                Some((lsp::uri_to_path(&location.uri)?, location.range))
                            })
                            .collect();
                        this.show_locations("references", targets, true, window, cx);
                    }
                    Err(error) => {
                        this.status = format!("Find References failed: {error}");
                        cx.notify();
                    }
                }
            });
        });
    }

    pub(super) fn find_references(
        &mut self,
        _: &FindReferences,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.references(true, window, cx);
    }

    pub(super) fn find_references_without_declaration(
        &mut self,
        _: &FindReferencesWithoutDeclaration,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.references(false, window, cx);
    }

    /// Present every definition target (instead of only the first) when there are
    /// several; used by Go to Definition.
    pub(super) fn show_definition_targets(
        &mut self,
        response: GotoDefinitionResponse,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let targets = locations(response);
        if targets.is_empty() {
            return false;
        }
        self.show_locations("definitions", targets, false, window, cx);
        true
    }

    // ----- IDE-009: symbols -----

    /// Request document symbols for the active buffer; `then` receives the flattened
    /// symbols if the response is still current.
    pub(super) fn request_document_symbols(
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

    pub(super) fn go_to_symbol_in_file(
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

    pub(super) fn go_to_symbol_in_workspace(
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
    pub(super) fn update_workspace_symbols(&mut self, query: String, cx: &mut Context<Self>) {
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

    pub(super) fn open_workspace_symbol(
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

    // ----- IDE-005: rename -----

    pub(super) fn rename_symbol(
        &mut self,
        _: &RenameSymbol,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.rename {
            return self.unsupported("Rename Symbol", &target, cx);
        }
        let text = self.buffers[target.ix].document.text().to_string();
        let word_at =
            |text: &str, position: lsp_types::Position| -> Option<(Range<usize>, String)> {
                let offset =
                    ide_core::text_edit::offset(text, position.line, position.character).ok()?;
                let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
                let start = text[..offset]
                    .char_indices()
                    .rev()
                    .take_while(|(_, c)| is_word(*c))
                    .last()
                    .map_or(offset, |(ix, _)| ix);
                let end = text[offset..]
                    .char_indices()
                    .find(|(_, c)| !is_word(*c))
                    .map_or(text.len(), |(ix, _)| offset + ix);
                (start < end).then(|| (start..end, text[start..end].to_string()))
            };
        if !target.features.prepare_rename {
            let Some((_, name)) = word_at(&text, target.position) else {
                self.status = "No symbol to rename here".into();
                cx.notify();
                return;
            };
            return self.prompt_rename(target, name, window, cx);
        }
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let Some(request) = self
            .servers
            .read(cx)
            .send::<PrepareRenameRequest>(target.language, position)
        else {
            return;
        };
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let prepared = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, "Rename", cx) {
                    return;
                }
                let text = this.buffers[target.ix].document.text().to_string();
                let name = match prepared {
                    Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
                        placeholder, ..
                    })) => Some(placeholder),
                    Ok(Some(PrepareRenameResponse::Range(range))) => {
                        let start = ide_core::text_edit::offset(
                            &text,
                            range.start.line,
                            range.start.character,
                        )
                        .ok();
                        let end =
                            ide_core::text_edit::offset(&text, range.end.line, range.end.character)
                                .ok();
                        start
                            .zip(end)
                            .and_then(|(start, end)| text.get(start..end))
                            .map(str::to_string)
                    }
                    Ok(Some(PrepareRenameResponse::DefaultBehavior { .. })) => {
                        word_at(&text, target.position).map(|(_, name)| name)
                    }
                    Ok(None) => {
                        this.status = "This symbol cannot be renamed".into();
                        cx.notify();
                        return;
                    }
                    Err(error) => {
                        this.status = format!("Cannot rename: {error}");
                        cx.notify();
                        return;
                    }
                };
                match name {
                    Some(name) => this.prompt_rename(target, name, window, cx),
                    None => {
                        this.status = "No symbol to rename here".into();
                        cx.notify();
                    }
                }
            });
        });
    }

    fn prompt_rename(
        &mut self,
        target: Target,
        current: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if window.has_active_dialog(cx) {
            return;
        }
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(current.clone())
                .validate(|text, _| !text.contains('\n'))
        });
        input.update(cx, |input, cx| {
            let len = current.len();
            input.select_byte_range(0..len, false, window, cx);
        });
        let shell = cx.entity().downgrade();
        let target = Rc::new(target);
        window.open_dialog(cx, {
            let input = input.clone();
            move |dialog, _, _| {
                let (shell, input, target) = (shell.clone(), input.clone(), target.clone());
                dialog
                    .title(format!("Rename {current}"))
                    .w(px(400.))
                    .child(Input::new(&input))
                    .confirm()
                    .button_props(DialogButtonProps::default().ok_text("Rename"))
                    .on_ok(move |_, window, cx| {
                        let name = input.read(cx).value().trim().to_string();
                        if name.is_empty() {
                            return false;
                        }
                        let _ = shell.update(cx, |shell, cx| {
                            shell.focus_editor(window, cx);
                            shell.perform_rename(&target, name, window, cx)
                        });
                        true
                    })
            }
        });
        input.update(cx, |input, cx| input.focus(window, cx));
    }

    fn perform_rename(
        &mut self,
        target: &Target,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(position) = Servers::position_params(&target.path, target.position) else {
            return;
        };
        let Some(request) = self.servers.read(cx).send::<Rename>(
            target.language,
            RenameParams {
                text_document_position: position,
                new_name: name.clone(),
                work_done_progress_params: Default::default(),
            },
        ) else {
            return;
        };
        let target = Target {
            ix: target.ix,
            id: target.id,
            path: target.path.clone(),
            language: target.language,
            position: target.position,
            origin: target.origin.clone(),
            features: target.features,
        };
        self.status = format!("Renaming to {name}…");
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let edit = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, "Rename", cx) {
                    return;
                }
                match edit {
                    Ok(Some(edit)) => this.request_workspace_edit(
                        edit,
                        format!("Rename to {name}"),
                        true,
                        window,
                        cx,
                    ),
                    Ok(None) => this.status = "The language server returned no rename edits".into(),
                    Err(error) => this.status = format!("Rename failed: {error}"),
                }
                cx.notify();
            });
        });
    }

    // ----- IDE-007: formatting -----

    pub(super) fn formatting_options(&self, ix: usize) -> FormattingOptions {
        let preferences = &self.buffers[ix].preferences;
        FormattingOptions {
            tab_size: preferences.tab_width as u32,
            insert_spaces: preferences.insert_spaces,
            properties: HashMap::new(),
            trim_trailing_whitespace: Some(preferences.trim_trailing_whitespace),
            insert_final_newline: Some(preferences.insert_final_newline),
            trim_final_newlines: None,
        }
    }

    /// Start a formatting request for the active buffer. Returns the request future and
    /// its target, or `None` (with a status message) when formatting is unavailable.
    pub(super) fn formatting_request(
        &mut self,
        selection_only: bool,
        quiet: bool,
        cx: &mut Context<Self>,
    ) -> Option<(Target, FormattingFuture)> {
        let target = if quiet {
            let ix = self.active;
            let path = self.buffers[ix].document.path()?.to_path_buf();
            let servers = self.servers.read(cx);
            servers.server_capabilities(self.buffers[ix].language)?;
            Target {
                ix,
                id: self.buffers[ix].id,
                language: self.buffers[ix].language,
                position: Default::default(),
                origin: servers.origin(&path),
                features: servers.features(self.buffers[ix].language),
                path,
            }
        } else {
            self.lsp_target(cx)?
        };
        let document = Servers::text_document(&target.path)?;
        let options = self.formatting_options(target.ix);
        let servers = self.servers.read(cx);
        if selection_only {
            if !target.features.range_formatting {
                if !quiet {
                    self.unsupported("Format Selection", &target, cx);
                }
                return None;
            }
            let range = self.source_range(target.ix, cx);
            let request = servers.send::<RangeFormatting>(
                target.language,
                DocumentRangeFormattingParams {
                    text_document: document,
                    range,
                    options,
                    work_done_progress_params: Default::default(),
                },
            )?;
            Some((target, Box::pin(request)))
        } else {
            if !target.features.formatting {
                if !quiet {
                    self.unsupported("Format Document", &target, cx);
                }
                return None;
            }
            let request = servers.send::<Formatting>(
                target.language,
                DocumentFormattingParams {
                    text_document: document,
                    options,
                    work_done_progress_params: Default::default(),
                },
            )?;
            Some((target, Box::pin(request)))
        }
    }

    /// Apply formatting edits if the document is unchanged since the request.
    pub(super) fn apply_formatting(
        &mut self,
        target: &Target,
        response: Result<Option<Vec<lsp_types::TextEdit>>, lsp::Error>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.still_current(target, "Formatting", cx) {
            return false;
        }
        let edits = match response {
            Ok(edits) => edits.unwrap_or_default(),
            Err(error) => {
                self.status = format!("Formatting failed: {error}");
                cx.notify();
                return false;
            }
        };
        if edits.is_empty() {
            self.status = "Already formatted".into();
            cx.notify();
            return true;
        }
        let Some(uri) = lsp::path_to_uri(&target.path) else {
            return false;
        };
        let edit = WorkspaceEdit {
            changes: Some([(uri, edits)].into_iter().collect()),
            ..Default::default()
        };
        match self
            .plan_workspace_edit(&edit, cx)
            .and_then(|plan| self.apply_plan(&plan, "Format", window, cx))
        {
            Ok(_) => {
                self.status = "Formatted".into();
                cx.notify();
                true
            }
            Err(error) => {
                self.status = format!("Formatting not applied: {error}");
                cx.notify();
                false
            }
        }
    }

    pub(super) fn format_document(
        &mut self,
        _: &FormatDocument,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format(false, window, cx);
    }

    pub(super) fn format_selection(
        &mut self,
        _: &FormatSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.format(true, window, cx);
    }

    fn format(&mut self, selection_only: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((target, request)) = self.formatting_request(selection_only, false, cx) else {
            return;
        };
        self.status = "Formatting…".into();
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.apply_formatting(&target, response, window, cx);
            });
        });
    }

    // ----- IDE-006: code actions -----

    fn request_code_actions(
        &mut self,
        only: Option<Vec<lsp_types::CodeActionKind>>,
        what: &'static str,
        auto_apply_single: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(target) = self.lsp_target(cx) else {
            return;
        };
        if !target.features.code_actions {
            return self.unsupported(what, &target, cx);
        }
        let Some(document) = Servers::text_document(&target.path) else {
            return;
        };
        let range = self.source_range(target.ix, cx);
        let diagnostics: Vec<lsp_types::Diagnostic> = self
            .lsp_diagnostics
            .get(&target.path)
            .map(|list| {
                list.iter()
                    .filter(|diagnostic| {
                        diagnostic.range.start <= range.end && range.start <= diagnostic.range.end
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let Some(request) = self.servers.read(cx).send::<CodeActionRequest>(
            target.language,
            CodeActionParams {
                text_document: document,
                range,
                context: CodeActionContext {
                    diagnostics,
                    only,
                    trigger_kind: Some(lsp_types::CodeActionTriggerKind::INVOKED),
                },
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            },
        ) else {
            return;
        };
        self.status = format!("{what}…");
        cx.notify();
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let response = request.await;
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.still_current(&target, what, cx) || this.buffer().id != target.id {
                    return;
                }
                let actions = match response {
                    Ok(actions) => actions.unwrap_or_default(),
                    Err(error) => {
                        this.status = format!("{what} failed: {error}");
                        cx.notify();
                        return;
                    }
                };
                if actions.is_empty() {
                    this.status = format!("No {} available here", what.to_lowercase());
                    cx.notify();
                    return;
                }
                let enabled = actions
                    .iter()
                    .filter(|action| !matches!(action, CodeActionOrCommand::CodeAction(action) if action.disabled.is_some()))
                    .count();
                this.code_actions = Some((target, actions));
                if auto_apply_single && enabled == 1 {
                    let index = this
                        .code_actions
                        .as_ref()
                        .and_then(|(_, actions)| {
                            actions.iter().position(|action| {
                                !matches!(action, CodeActionOrCommand::CodeAction(action) if action.disabled.is_some())
                            })
                        })
                        .unwrap_or(0);
                    return this.run_code_action(index, window, cx);
                }
                let items = this
                    .code_actions
                    .as_ref()
                    .map(|(_, actions)| {
                        actions
                            .iter()
                            .enumerate()
                            .map(|(ix, action)| {
                                let (title, detail) = match action {
                                    CodeActionOrCommand::Command(command) => (command.title.clone(), String::new()),
                                    CodeActionOrCommand::CodeAction(action) => (
                                        format!(
                                            "{}{}",
                                            if action.is_preferred == Some(true) { "★ " } else { "" },
                                            action.title
                                        ),
                                        match &action.disabled {
                                            Some(disabled) => format!("disabled: {}", disabled.reason),
                                            None => action
                                                .kind
                                                .as_ref()
                                                .map(|kind| kind.as_str().to_string())
                                                .unwrap_or_default(),
                                        },
                                    ),
                                };
                                PickerItem {
                                    filter: title.clone(),
                                    label: title,
                                    detail,
                                    depth: 0,
                                    action: PickerAction::CodeAction(ix),
                                }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                this.show_picker(PickerKind::CodeActions, what.to_string(), items, window, cx);
            });
        });
    }

    pub(super) fn quick_fix(&mut self, _: &QuickFix, window: &mut Window, cx: &mut Context<Self>) {
        self.request_code_actions(None, "Quick Fix", false, window, cx);
    }

    pub(super) fn refactor(&mut self, _: &Refactor, window: &mut Window, cx: &mut Context<Self>) {
        self.request_code_actions(
            Some(vec![lsp_types::CodeActionKind::REFACTOR]),
            "Refactor",
            false,
            window,
            cx,
        );
    }

    pub(super) fn source_action(
        &mut self,
        _: &SourceAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_code_actions(
            Some(vec![lsp_types::CodeActionKind::SOURCE]),
            "Source Action",
            false,
            window,
            cx,
        );
    }

    pub(super) fn organize_imports(
        &mut self,
        _: &OrganizeImports,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.request_code_actions(
            Some(vec![lsp_types::CodeActionKind::SOURCE_ORGANIZE_IMPORTS]),
            "Organize Imports",
            true,
            window,
            cx,
        );
    }

    /// Run the chosen code action: resolve it lazily when it has no edit, apply its edit
    /// through the shared applier, then execute its command.
    pub(super) fn run_code_action(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((target, actions)) = self.code_actions.take() else {
            return;
        };
        let Some(action) = actions.get(index).cloned() else {
            return;
        };
        if !self.servers.read(cx).is_current(&target.origin) {
            self.status = "The document changed; run the code action request again".into();
            cx.notify();
            return;
        }
        match action {
            CodeActionOrCommand::Command(command) => {
                self.run_command_for(&target, command, window, cx)
            }
            CodeActionOrCommand::CodeAction(action) => {
                if let Some(disabled) = &action.disabled {
                    self.status = format!("{} is unavailable: {}", action.title, disabled.reason);
                    cx.notify();
                    return;
                }
                let needs_resolve = action.edit.is_none() && action.data.is_some();
                if needs_resolve && target.features.resolve_code_actions {
                    let Some(request) = self
                        .servers
                        .read(cx)
                        .send::<CodeActionResolveRequest>(target.language, action.clone())
                    else {
                        return;
                    };
                    self.lsp_task = cx.spawn_in(window, async move |this, cx| {
                        let resolved = request.await;
                        let _ = this.update_in(cx, |this, window, cx| match resolved {
                            Ok(resolved) => this.finish_code_action(&target, resolved, window, cx),
                            Err(error) => {
                                this.status =
                                    format!("Could not resolve {}: {error}", action.title);
                                cx.notify();
                            }
                        });
                    });
                } else {
                    self.finish_code_action(&target, action, window, cx);
                }
            }
        }
    }

    fn finish_code_action(
        &mut self,
        target: &Target,
        action: lsp_types::CodeAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.still_current(target, &action.title, cx) {
            return;
        }
        if let Some(edit) = action.edit.clone() {
            let plan = self.plan_workspace_edit(&edit, cx);
            match plan {
                Ok(plan) => match self.apply_plan(&plan, &action.title, window, cx) {
                    Ok(summary) => self.status = summary,
                    Err(error) => {
                        self.status = error;
                        cx.notify();
                        return;
                    }
                },
                Err(error) => {
                    self.status = format!("{} not applied: {error}", action.title);
                    cx.notify();
                    return;
                }
            }
        }
        if let Some(command) = action.command {
            self.run_command_for(target, command, window, cx);
        } else if action.edit.is_none() {
            self.status = format!("{} has nothing to apply", action.title);
        }
        cx.notify();
    }

    pub(super) fn run_command_for(
        &mut self,
        target: &Target,
        command: lsp_types::Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !target.features.execute_command {
            self.status = format!("The server cannot run {}", command.title);
            cx.notify();
            return;
        }
        let Some(request) = self.servers.read(cx).send::<ExecuteCommand>(
            target.language,
            lsp_types::ExecuteCommandParams {
                command: command.command.clone(),
                arguments: command.arguments.clone().unwrap_or_default(),
                work_done_progress_params: Default::default(),
            },
        ) else {
            return;
        };
        let title = command.title.clone();
        self.status = format!("Running {title}…");
        cx.notify();
        // Edits produced by the command arrive as workspace/applyEdit requests.
        self.lsp_task = cx.spawn_in(window, async move |this, cx| {
            let result = request.await;
            let _ = this.update(cx, |this, cx| {
                if let Err(error) = result {
                    this.status = format!("{title} failed: {error}");
                } else if this.status.starts_with("Running ") {
                    this.status = format!("Ran {title}");
                }
                cx.notify();
            });
        });
    }
}
