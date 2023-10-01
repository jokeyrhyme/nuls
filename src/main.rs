#![deny(clippy::all, clippy::pedantic, unsafe_code)]

use std::{borrow::Cow, ffi::OsStr, sync::RwLock};

use error::map_err_to_parse_error;
use nu::{
    convert_position, convert_span, find_line_breaks, run_compiler, IdeComplete, IdeGotoDef,
    IdeHover, IdeSettings,
};

use tower_lsp::jsonrpc::Result;
#[allow(clippy::wildcard_imports)]
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod error;
mod nu;

#[derive(Debug)]
struct Backend {
    client: Client,
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
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
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
                // TODO: what do we do when the client doesn't support UTF-8 ?
                position_encoding: Some(PositionEncodingKind::UTF8),
                // TODO: improve performance by avoiding re-reading files over and over
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::NONE,
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
        let file_path = params
            .text_document_position
            .text_document
            .uri
            .to_file_path()
            .map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot convert URI to filesystem path: {e:?}",
                ))
            })?;
        let text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read file: {e:?}"))
        })?;
        let offset = convert_position(params.text_document_position.position, &text);

        let ide_settings = self.get_document_settings()?;
        let output = run_compiler(
            &text,
            vec![
                OsStr::new("--ide-complete"),
                OsStr::new(&format!("{offset}")),
            ],
            ide_settings,
            params.text_document_position.text_document.uri,
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
        let file_path = params
            .text_document_position_params
            .text_document
            .uri
            .to_file_path()
            .map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot convert URI to filesystem path: {e:?}",
                ))
            })?;
        let text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read file: {e:?}"))
        })?;
        let offset = convert_position(params.text_document_position_params.position, &text);

        let ide_settings = self.get_document_settings()?;
        let output = run_compiler(
            &text,
            vec![
                OsStr::new("--ide-goto-def"),
                OsStr::new(&format!("{offset}")),
            ],
            ide_settings,
            params.text_document_position_params.text_document.uri,
        )
        .await?;

        let goto_def: IdeGotoDef =
            serde_json::from_slice(output.stdout.as_bytes()).map_err(|e| {
                map_err_to_parse_error(e, format!("cannot parse response from {}", output.cmdline))
            })?;

        let line_breaks = find_line_breaks(&text);

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

        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: Url::from_file_path(goto_def.file).map_err(|_| {
                let mut err = tower_lsp::jsonrpc::Error::parse_error();
                err.message = Cow::from(
                    "failed to parse filesystem path in response from `nu --ide-goto-def`",
                );
                err
            })?,
            range: Range {
                start: convert_span(goto_def.start, &line_breaks),
                end: convert_span(goto_def.end, &line_breaks),
            },
        })))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let file_path = params
            .text_document_position_params
            .text_document
            .uri
            .to_file_path()
            .map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot convert URI to filesystem path: {e:?}",
                ))
            })?;
        let text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read file: {e:?}"))
        })?;
        let offset = convert_position(params.text_document_position_params.position, &text);

        let ide_settings = self.get_document_settings()?;
        let output = run_compiler(
            &text,
            vec![OsStr::new("--ide-hover"), OsStr::new(&format!("{offset}"))],
            ide_settings,
            params.text_document_position_params.text_document.uri,
        )
        .await?;

        let hover: IdeHover = serde_json::from_slice(output.stdout.as_bytes()).map_err(|e| {
            map_err_to_parse_error(e, format!("cannot parse response from {}", output.cmdline))
        })?;

        let line_breaks = find_line_breaks(&text);
        let range = hover.span.as_ref().map(|span| Range {
            start: convert_span(span.start, &line_breaks),
            end: convert_span(span.end, &line_breaks),
        });

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(hover.hover)),
            range,
        }))
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
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend {
        client,
        ide_settings: RwLock::new(IdeSettings::default()),
    });
    Server::new(stdin, stdout, socket).serve(service).await;
}
