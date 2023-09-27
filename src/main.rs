#![deny(clippy::all, unsafe_code)]

use std::borrow::Cow;

use nu::{convert_position, IdeComplete, IdeGotoDef, IdeHover};
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
    async fn initialize(&self, _: InitializeParams) -> Result<InitializeResult> {
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

    async fn initialized(&self, _: InitializedParams) {
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
        let index = convert_position(&params.text_document_position.position, &text);

        // TODO: call nushell Rust code directly instead of via separate process
        let output = tokio::process::Command::new("nu")
            .args([
                "--ide-complete",
                &format!("{}", index),
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
        let index = convert_position(&params.text_document_position_params.position, &text);

        // TODO: call nushell Rust code directly instead of via separate process
        let output = tokio::process::Command::new("nu")
            .args([
                "--ide-goto-def",
                &format!("{}", index),
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

        Ok(Some(GotoDefinitionResponse::Scalar(Location {
            uri: Url::from_file_path(goto_def.file).map_err(|e| {
                let mut err = tower_lsp::jsonrpc::Error::parse_error();
                err.data = Some(Value::String(format!("{:?}", e)));
                err.message = Cow::from(
                    "failed to parse filesystem path in response from `nu --ide-goto-def`",
                );
                err
            })?,
            // TODO: port convertSpan() from vscode-nushell-lang
            range: Range::default(),
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
        let index = convert_position(&params.text_document_position_params.position, &text);

        // TODO: call nushell Rust code directly instead of via separate process
        let output = tokio::process::Command::new("nu")
            .args([
                "--ide-hover",
                &format!("{}", index),
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

        Ok(Some(Hover {
            contents: HoverContents::Scalar(MarkedString::String(hover.hover)),
            // TODO: port convertSpan() from vscode-nushell-lang
            range: None,
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
