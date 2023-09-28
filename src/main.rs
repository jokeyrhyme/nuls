#![deny(clippy::all, unsafe_code)]

use std::borrow::Cow;

use nu::{convert_position, convert_span, find_line_breaks, IdeComplete, IdeGotoDef, IdeHover};
use serde_json::Value;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod nu;

#[derive(Debug)]
struct Backend {
    client: Client,
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
                    "cannot convert URI to filesystem path: {:?}",
                    e
                ))
            })?;
        let text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read file: {}", e))
        })?;
        let offset = convert_position(&params.text_document_position.position, &text);

        // TODO: call nushell Rust code directly instead of via separate process
        let output = tokio::process::Command::new("nu")
            .args([
                "--ide-complete",
                &format!("{}", offset),
                &format!("{}", file_path.display()),
            ])
            .output()
            .await
            .map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::internal_error();
                err.data = Some(Value::String(format!("{:?}", e)));
                err.message = Cow::from("`nu --ide-complete` failed");
                err
            })?;

        let complete: IdeComplete =
            serde_json::from_slice(output.stdout.as_slice()).map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::parse_error();
                err.data = Some(Value::String(format!("{:?}", e)));
                err.message = Cow::from("failed to parse response from `nu --ide-complete`");
                err
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
                    "cannot convert URI to filesystem path: {:?}",
                    e
                ))
            })?;
        let text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read file: {}", e))
        })?;
        let offset = convert_position(&params.text_document_position_params.position, &text);

        // TODO: call nushell Rust code directly instead of via separate process
        let output = tokio::process::Command::new("nu")
            .args([
                "--ide-goto-def",
                &format!("{}", offset),
                &format!("{}", file_path.display()),
            ])
            .output()
            .await
            .map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::internal_error();
                err.data = Some(Value::String(format!("{:?}", e)));
                err.message = Cow::from("`nu --ide-goto-def` failed");
                err
            })?;

        let goto_def: IdeGotoDef =
            serde_json::from_slice(output.stdout.as_slice()).map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::parse_error();
                err.data = Some(Value::String(format!("{:?}", e)));
                err.message = Cow::from("failed to parse response from `nu --ide-goto-def`");
                err
            })?;

        let line_breaks = find_line_breaks(&text);

        if matches!(
            goto_def.file.to_str(),
            None | Some("") | Some("__prelude__")
        ) {
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
            uri: Url::from_file_path(goto_def.file).map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::parse_error();
                err.data = Some(Value::String(format!("{:?}", e)));
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
                    "cannot convert URI to filesystem path: {:?}",
                    e
                ))
            })?;
        let text = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!("cannot read file: {}", e))
        })?;
        let offset = convert_position(&params.text_document_position_params.position, &text);

        // TODO: call nushell Rust code directly instead of via separate process
        let output = tokio::process::Command::new("nu")
            .args([
                "--ide-hover",
                &format!("{}", offset),
                &format!("{}", file_path.display()),
            ])
            .output()
            .await
            .map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::internal_error();
                err.data = Some(Value::String(format!("{:?}", e)));
                err.message = Cow::from("`nu --ide-hover` failed");
                err
            })?;

        let hover: IdeHover = serde_json::from_slice(output.stdout.as_slice()).map_err(|e| {
            let mut err = tower_lsp::jsonrpc::Error::parse_error();
            err.data = Some(Value::String(format!("{:?}", e)));
            err.message = Cow::from("failed to parse response from `nu --ide-hover`");
            err
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
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend { client });
    Server::new(stdin, stdout, socket).serve(service).await;
}
