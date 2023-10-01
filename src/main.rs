#![deny(clippy::all, clippy::pedantic, unsafe_code)]

use std::{borrow::Cow, ffi::OsStr, sync::RwLock};

use error::map_err_to_parse_error;
use lsp_textdocument::TextDocuments;
use nu::{run_compiler, IdeComplete, IdeGotoDef, IdeHover, IdeSettings};

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
    client: Client,
    documents: RwLock<TextDocuments>,
    ide_settings: RwLock<IdeSettings>,
}
impl Backend {
    fn get_document_settings(&self) -> Result<IdeSettings> {
        Ok(self
            .ide_settings
            .read()
            .map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read settings: {e:?}"))
            })?
            .clone())
    }

    fn try_did_change(&self, params: DidChangeTextDocumentParams) -> Result<()> {
        let mut documents = self.documents.write().map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot write to document cache: {e:?}"
            ))
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
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot write to document cache: {e:?}"
            ))
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
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot write to document cache: {e:?}"
            ))
        })?;
        let params = serde_json::to_value(params).map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot convert client parameters: {e:?}"
            ))
        })?;
        documents.listen(<DidOpenTextDocument as Notification>::METHOD, &params);
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
        if let Err(e) = self.try_did_open(params) {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        }
    }

    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
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
        let (text, offset) = {
            let documents = self.documents.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read from document cache: {e:?}"
                ))
            })?;
            let doc =
                documents
                    .get_document(&uri)
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "{uri} not found in document cache"
                    )))?;

            (
                String::from(doc.get_content(None)),
                doc.offset_at(params.text_document_position.position),
            )
        };

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
        let (text, offset) = {
            let documents = self.documents.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read from document cache: {e:?}"
                ))
            })?;
            let doc =
                documents
                    .get_document(&uri)
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "{uri} not found in document cache"
                    )))?;

            (
                String::from(doc.get_content(None)),
                doc.offset_at(params.text_document_position_params.position),
            )
        };

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

        let range = {
            let documents = self.documents.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read from document cache: {e:?}"
                ))
            })?;
            let doc =
                documents
                    .get_document(&uri)
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "{uri} not found in document cache"
                    )))?;

            Range {
                start: doc.position_at(goto_def.start),
                end: doc.position_at(goto_def.end),
            }
        };

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
        let (text, offset) = {
            let documents = self.documents.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read from document cache: {e:?}"
                ))
            })?;
            let doc =
                documents
                    .get_document(&uri)
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "{uri} not found in document cache"
                    )))?;

            (
                String::from(doc.get_content(None)),
                doc.offset_at(params.text_document_position_params.position),
            )
        };

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

        let range = {
            let documents = self.documents.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read from document cache: {e:?}"
                ))
            })?;
            let doc =
                documents
                    .get_document(&uri)
                    .ok_or(tower_lsp::jsonrpc::Error::invalid_params(format!(
                        "{uri} not found in document cache"
                    )))?;

            hover.span.as_ref().map(|span| Range {
                start: doc.position_at(span.start),
                end: doc.position_at(span.end),
            })
        };

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
        client,
        documents: RwLock::new(TextDocuments::new()),
        ide_settings: RwLock::new(IdeSettings::default()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
