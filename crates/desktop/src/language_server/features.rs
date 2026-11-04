//! Generic requests and capability queries used by the language features
//! (rename, code actions, formatting, references, symbols, completion, signature help).

use lsp_types::{OneOf, ServerCapabilities, request::Request};

use super::*;

/// Which optional server features are available for a language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Features {
    pub rename: bool,
    pub prepare_rename: bool,
    pub code_actions: bool,
    pub resolve_code_actions: bool,
    pub formatting: bool,
    pub range_formatting: bool,
    pub references: bool,
    pub implementation: bool,
    pub type_definition: bool,
    pub document_symbols: bool,
    pub workspace_symbols: bool,
    pub resolve_workspace_symbols: bool,
    pub signature_help: bool,
    pub completion: bool,
    pub resolve_completion: bool,
    pub execute_command: bool,
}

fn enabled<T>(value: &Option<OneOf<bool, T>>) -> bool {
    matches!(value, Some(OneOf::Left(true) | OneOf::Right(_)))
}

impl Features {
    pub fn of(capabilities: &ServerCapabilities) -> Self {
        use lsp_types::{
            CodeActionProviderCapability, ImplementationProviderCapability,
            TypeDefinitionProviderCapability,
        };
        Self {
            rename: enabled(&capabilities.rename_provider),
            prepare_rename: matches!(
                &capabilities.rename_provider,
                Some(OneOf::Right(options)) if options.prepare_provider == Some(true)
            ),
            code_actions: matches!(
                capabilities.code_action_provider,
                Some(
                    CodeActionProviderCapability::Simple(true)
                        | CodeActionProviderCapability::Options(_)
                )
            ),
            resolve_code_actions: matches!(
                &capabilities.code_action_provider,
                Some(CodeActionProviderCapability::Options(options)) if options.resolve_provider == Some(true)
            ),
            formatting: enabled(&capabilities.document_formatting_provider),
            range_formatting: enabled(&capabilities.document_range_formatting_provider),
            references: enabled(&capabilities.references_provider),
            implementation: matches!(
                capabilities.implementation_provider,
                Some(
                    ImplementationProviderCapability::Simple(true)
                        | ImplementationProviderCapability::Options(_)
                )
            ),
            type_definition: matches!(
                capabilities.type_definition_provider,
                Some(
                    TypeDefinitionProviderCapability::Simple(true)
                        | TypeDefinitionProviderCapability::Options(_)
                )
            ),
            document_symbols: enabled(&capabilities.document_symbol_provider),
            workspace_symbols: enabled(&capabilities.workspace_symbol_provider),
            resolve_workspace_symbols: matches!(
                &capabilities.workspace_symbol_provider,
                Some(OneOf::Right(options)) if options.resolve_provider == Some(true)
            ),
            signature_help: capabilities.signature_help_provider.is_some(),
            completion: capabilities.completion_provider.is_some(),
            resolve_completion: capabilities
                .completion_provider
                .as_ref()
                .and_then(|options| options.resolve_provider)
                .unwrap_or(false),
            execute_command: capabilities.execute_command_provider.is_some(),
        }
    }
}

/// The source document a request was made for, to detect stale responses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestOrigin {
    pub path: PathBuf,
    pub version: Option<i32>,
}

impl Servers {
    /// The version last sent to the server for an open document.
    pub fn version(&self, path: &Path) -> Option<i32> {
        self.documents.get(path).map(|document| document.version)
    }

    pub fn origin(&self, path: &Path) -> RequestOrigin {
        RequestOrigin {
            path: path.to_path_buf(),
            version: self.version(path),
        }
    }

    /// Whether `origin` still describes the current document version.
    pub fn is_current(&self, origin: &RequestOrigin) -> bool {
        self.version(&origin.path) == origin.version
    }

    pub fn server_capabilities(&self, language: &str) -> Option<ServerCapabilities> {
        self.client(language)?.capabilities()
    }

    pub fn features(&self, language: &str) -> Features {
        self.server_capabilities(language)
            .map(|capabilities| Features::of(&capabilities))
            .unwrap_or_default()
    }

    /// Send any request to the server for `language`, when it is ready.
    pub fn send<R: Request>(
        &self,
        language: &str,
        params: R::Params,
    ) -> Option<impl Future<Output = Result<R::Result, lsp::Error>> + use<R>> {
        Some(self.client(language)?.request::<R>(params))
    }

    /// Answer a forwarded server request.
    pub fn respond(
        &self,
        server: &str,
        id: serde_json::Value,
        result: Result<serde_json::Value, String>,
    ) {
        if let Some(server) = self.servers.get(server) {
            server.client.respond(id, result);
        }
    }

    pub fn text_document(path: &Path) -> Option<TextDocumentIdentifier> {
        Some(TextDocumentIdentifier {
            uri: lsp::path_to_uri(path)?,
        })
    }

    pub fn position_params(path: &Path, position: Position) -> Option<TextDocumentPositionParams> {
        Some(TextDocumentPositionParams {
            text_document: Self::text_document(path)?,
            position,
        })
    }

    /// Whether any ready server lists `character` as a completion trigger.
    pub fn is_trigger_character(&self, character: char) -> bool {
        self.servers.values().any(|server| {
            server.state == State::Ready
                && server
                    .client
                    .capabilities()
                    .and_then(|capabilities| capabilities.completion_provider)
                    .and_then(|options| options.trigger_characters)
                    .is_some_and(|triggers| {
                        triggers.iter().any(|trigger| trigger.ends_with(character))
                    })
        })
    }
}
