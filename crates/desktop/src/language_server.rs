use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    rc::Rc,
};

use anyhow::Result;
use gpui::{App, Context, Entity, EventEmitter, Task, WeakEntity, Window};
use gpui_component::input::{HoverProvider, InputState, Position, Rope};
use lsp_types::{
    CompletionItem, CompletionParams, CompletionResponse, GotoDefinitionParams,
    GotoDefinitionResponse, Hover, HoverContents, HoverParams, MarkupContent, MarkupKind,
    TextDocumentIdentifier, TextDocumentPositionParams,
};

use crate::{navigation, settings::LanguageServer, syntax};

#[cfg(test)]
#[path = "language_server_tests.rs"]
mod tests;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    Starting,
    Ready,
    Failed(String),
}

impl State {
    pub fn label(&self) -> &str {
        match self {
            Self::Starting => "starting…",
            Self::Ready => "ready",
            Self::Failed(reason) => reason,
        }
    }
}

pub enum ServerEvent {
    Diagnostics {
        path: PathBuf,
        diagnostics: Vec<lsp_types::Diagnostic>,
    },
    Ready,
    Status(String),
}

struct Server {
    client: lsp::Client,
    state: State,
    _events: Task<()>,
}

struct Document {
    server: String,
    language: &'static str,
    version: i32,
}

#[derive(Default)]
pub struct Servers {
    root: Option<PathBuf>,
    configs: Vec<LanguageServer>,
    servers: HashMap<String, Server>,
    documents: HashMap<PathBuf, Document>,
    failures: HashMap<String, String>,
}

impl EventEmitter<ServerEvent> for Servers {}

impl Servers {
    pub fn set_root(&mut self, root: Option<PathBuf>, cx: &mut Context<Self>) {
        if self.root == root {
            return;
        }
        self.root = root;
        self.stop(cx);
    }

    pub fn set_configs(&mut self, configs: Vec<LanguageServer>, cx: &mut Context<Self>) {
        if self.configs == configs {
            return;
        }
        self.configs = configs;
        self.stop(cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.servers.clear();
        self.documents.clear();
        self.failures.clear();
        cx.notify();
    }

    pub fn state(&self, language: &str) -> Option<(String, State)> {
        let name = self.config(language)?.program.clone();
        let state = match (self.servers.get(&name), self.failures.get(&name)) {
            (Some(server), _) => server.state.clone(),
            (None, Some(reason)) => State::Failed(reason.clone()),
            (None, None) => State::Starting,
        };
        Some((name, state))
    }

    fn config(&self, language: &str) -> Option<&LanguageServer> {
        self.configs.iter().find(|config| config.serves(language))
    }

    fn client(&self, language: &str) -> Option<&lsp::Client> {
        let name = &self.config(language)?.program;
        let server = self.servers.get(name)?;
        (server.state == State::Ready && server.client.is_running()).then_some(&server.client)
    }

    fn start(&mut self, language: &'static str, cx: &mut Context<Self>) -> Option<String> {
        let root = self.root.clone()?;
        let config = self.config(language)?.clone();
        let name = config.program.clone();
        if self.servers.contains_key(&name) {
            return Some(name);
        }
        if self.failures.contains_key(&name) {
            return None;
        }
        let options = lsp::Options {
            program: config.program.clone(),
            args: config.args.clone(),
            env: config.env.clone(),
            root,
            initialization_options: config.initialization_options.clone(),
            settings: config.settings.clone(),
        };
        let client = match lsp::Client::start(&options) {
            Ok(client) => client,
            Err(error) => {
                self.failures.insert(name.clone(), error.to_string());
                cx.emit(ServerEvent::Status(format!("{name}: {error}")));
                return None;
            }
        };
        let events = client.events();
        let owner = name.clone();
        let pump = cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                let stopped = matches!(event, lsp::Event::Stopped(_));
                if this
                    .update(cx, |this, cx| this.handle(&owner, event, cx))
                    .is_err()
                    || stopped
                {
                    break;
                }
            }
        });
        let handshake = client.initialize(&options);
        let owner = name.clone();
        cx.spawn(async move |this, cx| {
            let outcome = handshake.await;
            let _ = this.update(cx, |this, cx| {
                let Some(server) = this.servers.get_mut(&owner) else {
                    return;
                };
                match outcome {
                    Ok(_) => {
                        server.state = State::Ready;
                        cx.emit(ServerEvent::Ready);
                    }
                    Err(error) => {
                        server.state = State::Failed(error.to_string());
                        cx.emit(ServerEvent::Status(format!("{owner}: {error}")));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        self.servers.insert(
            name.clone(),
            Server {
                client,
                state: State::Starting,
                _events: pump,
            },
        );
        Some(name)
    }

    fn handle(&mut self, name: &str, event: lsp::Event, cx: &mut Context<Self>) {
        match event {
            lsp::Event::Diagnostics { path, diagnostics } => {
                if self.documents.contains_key(&path) {
                    cx.emit(ServerEvent::Diagnostics { path, diagnostics });
                }
            }
            lsp::Event::Log(message) => {
                let message = message.lines().next().unwrap_or_default().trim();
                if !message.is_empty() {
                    cx.emit(ServerEvent::Status(format!("{name}: {message}")));
                }
            }
            lsp::Event::Stopped(reason) => {
                let log = self
                    .servers
                    .get(name)
                    .map(|server| server.client.log())
                    .unwrap_or_default();
                let detail = log.lines().last().unwrap_or(&reason).trim().to_string();
                if let Some(server) = self.servers.get_mut(name) {
                    server.state = State::Failed(detail.clone());
                }
                self.documents.retain(|_, document| document.server != name);
                cx.emit(ServerEvent::Status(format!("{name} stopped: {detail}")));
                cx.notify();
            }
        }
    }

    pub fn sync(
        &mut self,
        path: &Path,
        language: &'static str,
        text: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(name) = self.start(language, cx) else {
            return;
        };
        if self.client(language).is_none() {
            return;
        }
        let Some(uri) = lsp::path_to_uri(path) else {
            return;
        };
        let opened = self.documents.contains_key(path);
        let document = self
            .documents
            .entry(path.to_path_buf())
            .or_insert(Document {
                server: name,
                language,
                version: 0,
            });
        document.version += 1;
        let version = document.version;
        let Some(client) = self.client(language) else {
            return;
        };
        if opened {
            client.notify::<lsp_types::notification::DidChangeTextDocument>(
                lsp_types::DidChangeTextDocumentParams {
                    text_document: lsp_types::VersionedTextDocumentIdentifier { uri, version },
                    content_changes: vec![lsp_types::TextDocumentContentChangeEvent {
                        range: None,
                        range_length: None,
                        text: text.to_string(),
                    }],
                },
            );
        } else {
            client.notify::<lsp_types::notification::DidOpenTextDocument>(
                lsp_types::DidOpenTextDocumentParams {
                    text_document: lsp_types::TextDocumentItem {
                        uri,
                        language_id: language_id(language).into(),
                        version,
                        text: text.to_string(),
                    },
                },
            );
        }
    }

    pub fn saved(&mut self, path: &Path, text: &str) {
        let Some(document) = self.documents.get(path) else {
            return;
        };
        let (Some(client), Some(uri)) = (self.client(document.language), lsp::path_to_uri(path))
        else {
            return;
        };
        client.notify::<lsp_types::notification::DidSaveTextDocument>(
            lsp_types::DidSaveTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
                text: Some(text.to_string()),
            },
        );
    }

    pub fn close(&mut self, path: &Path) {
        let Some(document) = self.documents.remove(path) else {
            return;
        };
        let (Some(client), Some(uri)) = (self.client(document.language), lsp::path_to_uri(path))
        else {
            return;
        };
        client.notify::<lsp_types::notification::DidCloseTextDocument>(
            lsp_types::DidCloseTextDocumentParams {
                text_document: TextDocumentIdentifier { uri },
            },
        );
    }

    pub fn hover(
        &self,
        path: &Path,
        language: &str,
        position: Position,
    ) -> Option<impl Future<Output = Option<Hover>> + use<>> {
        let request = self
            .client(language)?
            .request::<lsp_types::request::HoverRequest>(HoverParams {
                text_document_position_params: self.locate(path, position)?,
                work_done_progress_params: Default::default(),
            });
        Some(async move { request.await.ok().flatten() })
    }

    pub fn definition(
        &self,
        path: &Path,
        language: &str,
        position: Position,
    ) -> Option<impl Future<Output = Option<(PathBuf, Position)>> + use<>> {
        let request = self
            .client(language)?
            .request::<lsp_types::request::GotoDefinition>(GotoDefinitionParams {
                text_document_position_params: self.locate(path, position)?,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            });
        Some(async move { request.await.ok().flatten().and_then(first_location) })
    }

    pub fn completions(
        &self,
        path: &Path,
        language: &str,
        position: Position,
    ) -> Option<impl Future<Output = Vec<CompletionItem>> + use<>> {
        let request = self
            .client(language)?
            .request::<lsp_types::request::Completion>(CompletionParams {
                text_document_position: self.locate(path, position)?,
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
                context: None,
            });
        Some(async move {
            match request.await {
                Ok(Some(CompletionResponse::Array(items))) => items,
                Ok(Some(CompletionResponse::List(list))) => list.items,
                _ => Vec::new(),
            }
        })
    }

    fn locate(&self, path: &Path, position: Position) -> Option<TextDocumentPositionParams> {
        Some(TextDocumentPositionParams {
            text_document: TextDocumentIdentifier {
                uri: lsp::path_to_uri(path)?,
            },
            position,
        })
    }
}

fn first_location(response: GotoDefinitionResponse) -> Option<(PathBuf, Position)> {
    match response {
        GotoDefinitionResponse::Scalar(location) => {
            Some((lsp::uri_to_path(&location.uri)?, location.range.start))
        }
        GotoDefinitionResponse::Array(locations) => {
            let location = locations.into_iter().next()?;
            Some((lsp::uri_to_path(&location.uri)?, location.range.start))
        }
        GotoDefinitionResponse::Link(links) => {
            let link = links.into_iter().next()?;
            Some((
                lsp::uri_to_path(&link.target_uri)?,
                link.target_selection_range.start,
            ))
        }
    }
}

fn language_id(language: &str) -> &'static str {
    match language {
        "c" => "c",
        "c_sharp" => "csharp",
        "go" => "go",
        "java" => "java",
        "javascript" => "javascript",
        "jsx" => "javascriptreact",
        "rust" => "rust",
        "typescript" => "typescript",
        "tsx" => "typescriptreact",
        "zig" => "zig",
        _ => "plaintext",
    }
}

pub fn position_at(text: &str, offset: usize) -> Position {
    let offset = offset.min(text.len());
    let line = text[..offset].matches('\n').count() as u32;
    let start = text[..offset].rfind('\n').map_or(0, |ix| ix + 1);
    let character = text[start..offset].encode_utf16().count() as u32;
    Position::new(line, character)
}

pub struct EditorSupport {
    servers: WeakEntity<Servers>,
    path: Option<PathBuf>,
    language: &'static str,
    root: Option<PathBuf>,
}

pub fn attach(
    editor: &Entity<InputState>,
    servers: &Entity<Servers>,
    path: Option<PathBuf>,
    language: &'static str,
    cx: &mut App,
) {
    let support = Rc::new(EditorSupport {
        servers: servers.downgrade(),
        path,
        language,
        root: servers.read(cx).root.clone(),
    });
    editor.update(cx, |editor, _| {
        editor.lsp.hover_provider = Some(support);
    });
}

impl HoverProvider for EditorSupport {
    fn hover(
        &self,
        text: &Rope,
        offset: usize,
        _: &mut Window,
        cx: &mut App,
    ) -> Task<Result<Option<Hover>>> {
        let source = text.to_string();
        let request = match (self.servers.upgrade(), self.path.as_deref()) {
            (Some(servers), Some(path)) => {
                servers
                    .read(cx)
                    .hover(path, self.language, position_at(&source, offset))
            }
            _ => None,
        };
        let language = self.language;
        let root = self.root.clone();
        let origin = self.path.clone();
        cx.background_executor().spawn(async move {
            if let Some(request) = request
                && let Some(hover) = request.await
                && !is_empty(&hover)
            {
                return Ok(Some(hover));
            }
            Ok(declaration_hover(
                language,
                &source,
                offset,
                root.as_deref(),
                origin.as_deref(),
            ))
        })
    }
}

fn is_empty(hover: &Hover) -> bool {
    match &hover.contents {
        HoverContents::Scalar(lsp_types::MarkedString::String(text)) => text.trim().is_empty(),
        HoverContents::Scalar(_) => false,
        HoverContents::Array(items) => items.is_empty(),
        HoverContents::Markup(markup) => markup.value.trim().is_empty(),
    }
}

pub fn declaration_hover(
    language: &'static str,
    source: &str,
    offset: usize,
    root: Option<&Path>,
    origin: Option<&Path>,
) -> Option<Hover> {
    let (declaration, path) = match navigation::lookup(language, source, offset)? {
        navigation::Lookup::Found(name) => (
            navigation::summarize(language, source, name)?,
            origin.map(Path::to_path_buf),
        ),
        navigation::Lookup::Search(search) => {
            let hit =
                navigation::search_workspace(root?, origin, language, &search, &HashMap::new())?;
            let text = hit.document?.text().to_string();
            let language = syntax::language(Some(&hit.path));
            (
                navigation::summarize(language, &text, hit.name)?,
                Some(hit.path),
            )
        }
    };
    Some(markdown(language, &declaration, path.as_deref()))
}

fn markdown(language: &str, declaration: &navigation::Declaration, path: Option<&Path>) -> Hover {
    let mut value = format!("```{language}\n{}\n```", declaration.signature);
    if !declaration.documentation.is_empty() {
        value.push_str("\n\n");
        value.push_str(&declaration.documentation);
    }
    if let Some(name) = path.and_then(Path::file_name) {
        value.push_str(&format!(
            "\n\n`{}:{}`",
            name.to_string_lossy(),
            declaration.line
        ));
    }
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: None,
    }
}
