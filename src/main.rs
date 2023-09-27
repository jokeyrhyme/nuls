#![deny(clippy::all, unsafe_code)]

use std::borrow::Cow;

use serde::Deserialize;
use serde_json::Value;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

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
                // TODO: `nu --ide-check`
                // `nu --ide-complete`
                completion_provider: Some(CompletionOptions::default()),
                // TODO: `nu --ide-goto-def`
                // TODO: `nu --ide-hover`
                hover_provider: Some(HoverProviderCapability::Simple(true)),
                ..Default::default()
            },
            ..Default::default()
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

    async fn completion(&self, _params: CompletionParams) -> Result<Option<CompletionResponse>> {
        Ok(Some(CompletionResponse::Array(vec![
            CompletionItem::new_simple("Hello".to_string(), "Some detail".to_string()),
            CompletionItem::new_simple("Bye".to_string(), "More detail".to_string()),
        ])))
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

#[derive(Deserialize)]
struct IdeHover {
    hover: String,
    // span: Option<Range>,
}

// ported from https://github.com/nushell/vscode-nushell-lang
fn convert_position(position: &Position, text: &str) -> usize {
    let mut line = 0;
    let mut character = 0;
    let buffer = text.as_bytes();

    let mut i = 0;
    while i < buffer.len() {
        if line == position.line && character == position.character {
            return i;
        }

        if buffer[i] == 0x0a {
            line += 1;
            character = 0;
        } else {
            character += 1;
        }

        i += 1;
    }

    i
}

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(|client| Backend { client });
    Server::new(stdin, stdout, socket).serve(service).await;
}
