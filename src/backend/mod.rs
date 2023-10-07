use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{ffi::OsStr, sync::RwLock};

pub(crate) mod language_server;
use crate::{
    error::map_err_to_internal_error,
    nu::{run_compiler, IdeCheck, IdeCheckDiagnostic, IdeSettings},
};
use lsp_textdocument::{FullTextDocument, TextDocuments};

use tower_lsp::lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, Notification,
};
#[allow(clippy::wildcard_imports)]
use tower_lsp::lsp_types::*;
use tower_lsp::Client;
use tower_lsp::{jsonrpc::Result, lsp_types::notification::DidOpenTextDocument};

pub(crate) struct Backend {
    can_publish_diagnostics: OnceLock<bool>,
    client: Client,
    documents: RwLock<TextDocuments>,
    ide_settings: RwLock<IdeSettings>,
    last_validated: RwLock<Instant>,
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

    pub fn new(client: Client) -> Self {
        Self {
            can_publish_diagnostics: OnceLock::new(),
            client,
            documents: RwLock::new(TextDocuments::new()),
            ide_settings: RwLock::new(IdeSettings::default()),
            last_validated: RwLock::new(Instant::now()),
        }
    }

    async fn throttled_validate_document(&self, uri: &Url) -> Result<()> {
        // TODO: this is a quick imperfect hack, but eventually we probably want a thorough solution using threads/channels?
        // TODO: ensure that we validate at least once after the most recent throttling (i.e. debounce instead of throttle)
        let then = {
            *self.last_validated.read().map_err(|e| {
                map_err_to_internal_error(&e, format!("cannot read throttling marker: {e:?}"))
            })?
        };
        if then.elapsed() < Duration::from_millis(500) {
            return Ok(());
        }

        self.validate_document(uri).await?;

        let mut then = self.last_validated.write().map_err(|e| {
            map_err_to_internal_error(&e, format!("cannot write throttling marker: {e:?}"))
        })?;
        *then = Instant::now();
        Ok(())
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
