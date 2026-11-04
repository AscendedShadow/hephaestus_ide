//! Language-server commands: rename (IDE-005), code actions (IDE-006), formatting
//! (IDE-007), references and implementation/type navigation (IDE-008) and symbols
//! (IDE-009).
//!
//! Every request records the document version it was made for; a response that arrives
//! after the document changed (or for a buffer that is no longer active, where focus
//! would move unexpectedly) is discarded with an explanation instead of being applied.

mod code_actions;
mod formatting;
mod navigation;
mod rename;
mod symbols;

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

    // ----- IDE-009: symbols -----

    // ----- IDE-005: rename -----

    // ----- IDE-007: formatting -----

    // ----- IDE-006: code actions -----
}
