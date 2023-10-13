#![deny(clippy::all, clippy::pedantic, unsafe_code)]

mod backend;
mod deserialize;
mod error;
mod nu;
use backend::Backend;

use tower_lsp::{LspService, Server};

#[tokio::main]
async fn main() {
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();

    let (service, socket) = LspService::new(Backend::new);
    Server::new(stdin, stdout, socket).serve(service).await;
}
