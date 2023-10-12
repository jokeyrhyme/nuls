use std::{borrow::Cow, ffi::OsStr};

use crate::{
    backend::Backend,
    error::map_err_to_parse_error,
    nu::{run_compiler, IdeComplete, IdeGotoDef, IdeHover},
};

#[allow(clippy::wildcard_imports)]
use tower_lsp::lsp_types::*;
use tower_lsp::{jsonrpc::Result, lsp_types::notification::DidChangeConfiguration};
use tower_lsp::{lsp_types::notification::Notification, LanguageServer};

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri.clone();
        if let Err(e) = self.try_did_change(params) {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        }
        if let Err(e) = self.throttled_validate_document(&uri).await {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        };
    }

    async fn did_change_configuration(&self, params: DidChangeConfigurationParams) {
        if let Err(e) = self.try_did_change_configuration(params).await {
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
        let uri = params.text_document.uri.clone();
        if let Err(e) = self.try_did_open(params) {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        }
        if let Err(e) = self.validate_document(&uri).await {
            self.client
                .log_message(MessageType::ERROR, format!("{e:?}"))
                .await;
        };
    }

    async fn initialize(&self, params: InitializeParams) -> Result<InitializeResult> {
        // panic: this is the only place we `OnceLock::set`,
        // so we've entered strange territory if something else writes to them first

        self.can_change_configuration
            .set(matches!(
                params.capabilities.workspace,
                Some(WorkspaceClientCapabilities {
                    did_change_configuration: Some(_),
                    ..
                })
            ))
            .expect("server value initialized out of sequence");

        self.can_lookup_configuration
            .set(matches!(
                params.capabilities.workspace,
                Some(WorkspaceClientCapabilities {
                    configuration: Some(_),
                    ..
                })
            ))
            .expect("server value initialized out of sequence");

        self.can_publish_diagnostics
            .set(matches!(
                params.capabilities.text_document,
                Some(TextDocumentClientCapabilities {
                    publish_diagnostics: Some(_),
                    ..
                })
            ))
            .expect("server value initialized out of sequence");

        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                completion_provider: Some(CompletionOptions::default()),
                definition_provider: Some(OneOf::Left(true)),
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                inlay_hint_provider: Some(OneOf::Right(InlayHintServerCapabilities::Options(
                    InlayHintOptions {
                        resolve_provider: Some(false),
                        ..Default::default()
                    },
                ))),
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
        if *self.can_change_configuration.get().unwrap_or(&false) {
            let method = String::from(DidChangeConfiguration::METHOD);
            if let Err(e) = self
                .client
                .register_capability(vec![Registration {
                    id: method.clone(),
                    method,
                    register_options: None,
                }])
                .await
            {
                self.client
                    .log_message(
                        MessageType::INFO,
                        format!("unable to register capability: {e:?}"),
                    )
                    .await;
            };
        }

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

        let ide_settings = self.get_document_settings(&uri).await?;
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

        let complete = IdeComplete::try_from(output)?;

        Ok(Some(CompletionResponse::from(complete)))
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

        let ide_settings = self.get_document_settings(&uri).await?;
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
            uri: Url::from_file_path(goto_def.file).map_err(|()| {
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

        let ide_settings = self.get_document_settings(&uri).await?;
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

    async fn inlay_hint(&self, params: InlayHintParams) -> Result<Option<Vec<InlayHint>>> {
        let document_inlay_hints = self.document_inlay_hints.read().map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot read from inlay hints cache: {e:?}"
            ))
        })?;
        Ok(document_inlay_hints.get(&params.text_document.uri).cloned())
    }
}
