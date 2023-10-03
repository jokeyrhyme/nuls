#![deny(clippy::all, clippy::pedantic, unsafe_code)]

use std::sync::OnceLock;
use std::{borrow::Cow, ffi::OsStr, sync::RwLock};

use error::{map_err_to_internal_error, map_err_to_parse_error};
use lsp_textdocument::{FullTextDocument, TextDocuments};
use nu::{
    run_compiler, IdeCheck, IdeCheckDiagnostic, IdeComplete, IdeGotoDef, IdeHover, IdeSettings,
};

use tower_lsp::lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, Notification,
};
#[allow(clippy::wildcard_imports)]
use tower_lsp::lsp_types::*;
use tower_lsp::{jsonrpc::Result, lsp_types::notification::DidOpenTextDocument};
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod error;
mod nu;

struct Backend {
    can_publish_diagnostics: OnceLock<bool>,
    client: Client,
    documents: RwLock<TextDocuments>,
    ide_settings: RwLock<IdeSettings>,
}
impl Backend {
    fn for_document<T>(&self, uri: &Url, f: &dyn Fn(&FullTextDocument) -> T) -> Result<T> {
        let documents = self.documents.read().map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot read from document cache: {e:?}"
            ))
        })?;
        let doc = documents
            .get_document(uri)
            .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                "{uri} not found in document cache"
            )))?;

        Ok(f(doc))
    }

    fn get_document_settings(&self) -> Result<IdeSettings> {
        Ok(self
            .ide_settings
            .read()
            .map_err(|e| map_err_to_internal_error(&e, format!("cannot read settings: {e:?}")))?
            .clone())
    }

    fn try_did_change(&self, params: DidChangeTextDocumentParams) -> Result<()> {
        let mut documents = self.documents.write().map_err(|e| {
            map_err_to_internal_error(&e, format!("cannot write to document cache: {e:?}"))
        })?;
        let params = serde_json::to_value(params).map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot convert client parameters: {e:?}"
            ))
        })?;
        documents.listen(<DidChangeTextDocument as Notification>::METHOD, &params);
        Ok(())
    }

    fn try_did_close(&self, params: DidCloseTextDocumentParams) -> Result<()> {
        let mut documents = self.documents.write().map_err(|e| {
            map_err_to_internal_error(&e, format!("cannot write to document cache: {e:?}"))
        })?;
        let params = serde_json::to_value(params).map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot convert client parameters: {e:?}"
            ))
        })?;
        documents.listen(<DidCloseTextDocument as Notification>::METHOD, &params);
        Ok(())
    }

    fn try_did_open(&self, params: DidOpenTextDocumentParams) -> Result<()> {
        let mut documents = self.documents.write().map_err(|e| {
            map_err_to_internal_error(&e, format!("cannot write to document cache: {e:?}"))
        })?;
        let params = serde_json::to_value(params).map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot convert client parameters: {e:?}"
            ))
        })?;
        documents.listen(<DidOpenTextDocument as Notification>::METHOD, &params);
        Ok(())
    }

    async fn validate_document(&self, uri: &Url) -> Result<()> {
        let can_publish_diagnostics = self.can_publish_diagnostics.get_or_init(|| false);
        if !can_publish_diagnostics {
            self.client
                .log_message(
                    MessageType::INFO,
                    String::from("client did not report diagnostic capability"),
                )
                .await;
            return Ok(());
        }

        let text = self.for_document(uri, &|doc| String::from(doc.get_content(None)))?;

        let ide_settings = self.get_document_settings()?;
        let output =
            run_compiler(&text, vec![OsStr::new("--ide-check")], ide_settings, uri).await?;

        let ide_checks: Vec<IdeCheck> = output
            .stdout
            .lines()
            .filter_map(|l| serde_json::from_slice(l.as_bytes()).ok())
            .collect();

        let (diagnostics, version) = self.for_document(uri, &|doc| {
            (
                ide_checks
                    .iter()
                    .filter_map(|c| match c {
                        IdeCheck::Diagnostic(d) => Some(d),
                        IdeCheck::Hint(_) => None,
                    })
                    .map(|d| IdeCheckDiagnostic::to_diagnostic(d, doc, uri))
                    .collect::<Vec<_>>(),
                doc.version(),
            )
        })?;

        self.client
            .publish_diagnostics(uri.clone(), diagnostics, Some(version))
            .await;

        Ok(())
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        if let Err(e) = self.try_did_change(params) {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        }
        // TODO: trigger debounced `nu --ide-check`
    }

    async fn did_change_workspace_folders(&self, params: DidChangeWorkspaceFoldersParams) {
        self.client
            .log_message(
                MessageType::INFO,
                format!(
                    "workspace folders: added={:?}; removed={:?}",
                    params.event.added, params.event.removed
                ),
            )
            .await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        if let Err(e) = self.try_did_close(params) {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        }
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Err(e) = self.try_did_open(params) {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        }
        // TODO: trigger debounced `nu --ide-check` instead
        if let Err(e) = self.validate_document(&uri).await {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        };
    }

    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        if self
            .can_publish_diagnostics
            .set(matches!(
                params.capabilities.text_document,
                Some(TextDocumentClientCapabilities {
                    publish_diagnostics: Some(_),
                    ..
                })
            ))
            .is_err()
        {
            self.client
                .log_message(
                    MessageType::ERROR,
                    "diagnostic setting was initialized unexpectedly",
                )
                .await;
        }

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                // TODO: `nu --ide-ast`
                // `nu --ide-complete`
                completion_provider: Some(CompletionOptions::default()),
                // TODO: `nu --ide-check`
                // `nu --ide-goto-def`
                definition_provider: Some(OneOf::Left(true)),
                // `nu --ide-hover`
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                // TODO: what do we do when the client doesn't support UTF-16 ?
                // lsp-textdocument crate requires UTF-16
                position_encoding: Some(PositionEncodingKind::UTF16),
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::INCREMENTAL,
                )),
                workspace: Some(WorkspaceServerCapabilities {
                    workspace_folders: Some(WorkspaceFoldersServerCapabilities {
                        supported: Some(true),
                        change_notifications: Some(OneOf::Left(true)),
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
            server_info: Some(ServerInfo {
                name: String::from(env!("CARGO_PKG_NAME")),
                version: Some(String::from(env!("CARGO_PKG_VERSION"))),
            }),
        })
    }

    async fn initialized(&self, _params: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "server initialized!")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        self.client
            .log_message(MessageType::INFO, "server shutdown...!")
            .await;
        Ok(())
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let (text, offset) = self.for_document(&uri, &|doc| {
            (
                String::from(doc.get_content(None)),
                doc.offset_at(params.text_document_position.position),
            )
        })?;

        let ide_settings = self.get_document_settings()?;
        let output = run_compiler(
            &text,
            vec![
                OsStr::new("--ide-complete"),
                OsStr::new(&format!("{offset}")),
            ],
            ide_settings,
            &uri,
        )
        .await?;

        let complete: IdeComplete =
            serde_json::from_slice(output.stdout.as_bytes()).map_err(|e| {
                map_err_to_parse_error(e, format!("cannot parse response from {}", output.cmdline))
            })?;

        Ok(Some(CompletionResponse::Array(
            complete
                .completions
                .into_iter()
                .map(|c| CompletionItem::new_simple(c, String::new()))
                .collect(),
        )))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let (text, offset) = self.for_document(&uri, &|doc| {
            (
                String::from(doc.get_content(None)),
                doc.offset_at(params.text_document_position_params.position),
            )
        })?;

        let ide_settings = self.get_document_settings()?;
        let output = run_compiler(
            &text,
            vec![
                OsStr::new("--ide-goto-def"),
                OsStr::new(&format!("{offset}")),
            ],
            ide_settings,
            &uri,
        )
        .await?;

        let goto_def: IdeGotoDef =
            serde_json::from_slice(output.stdout.as_bytes()).map_err(|e| {
                map_err_to_parse_error(e, format!("cannot parse response from {}", output.cmdline))
            })?;

        if matches!(goto_def.file.to_str(), None | Some("" | "__prelude__")) {
            return Ok(None);
        }

        if !goto_def.file.exists() {
            self.client
                .log_message(
                    MessageType::ERROR,
                    format!("File {} does not exist", goto_def.file.display()),
                )
                .await;
            return Ok(None);
        }

        let range = self.for_document(&uri, &|doc| Range {
            start: doc.position_at(goto_def.start),
            end: doc.position_at(goto_def.end),
        })?;

        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: Url::from_file_path(goto_def.file).map_err(|_| {
                let mut err = tower_lsp::jsonrpc::Error::parse_error();
                err.message = Cow::from(
                    "failed to parse filesystem path in response from `nu --ide-goto-def`",
                );
                err
            })?,
            range,
        })))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let (text, offset) = self.for_document(&uri, &|doc| {
            (
                String::from(doc.get_content(None)),
                doc.offset_at(params.text_document_position_params.position),
            )
        })?;

        let ide_settings = self.get_document_settings()?;
        let output = run_compiler(
            &text,
            vec![OsStr::new("--ide-hover"), OsStr::new(&format!("{offset}"))],
            ide_settings,
            &uri,
        )
        .await?;

        let hover: IdeHover = serde_json::from_slice(output.stdout.as_bytes()).map_err(|e| {
            map_err_to_parse_error(e, format!("cannot parse response from {}", output.cmdline))
        })?;

        let range = self.for_document(&uri, &|doc| {
            hover.span.as_ref().map(|span| Range {
                start: doc.position_at(span.start),
                end: doc.position_at(span.end),
            })
        })?;

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(hover.hover)),
            range,
        }))
    }
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        can_publish_diagnostics: OnceLock::new(),
        client,
        documents: RwLock::new(TextDocuments::new()),
        ide_settings: RwLock::new(IdeSettings::default()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
