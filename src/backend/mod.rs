use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use std::{ffi::OsStr, sync::RwLock};

pub(crate) mod language_server;
use crate::{
    error::map_err_to_internal_error,
    nu::{run_compiler, IdeCheck, IdeCheckDiagnostic, IdeSettings},
};
use lsp_textdocument::{FullTextDocument, TextDocuments};

use serde::Deserialize;
use tower_lsp::lsp_types::notification::{
    DidChangeTextDocument, DidCloseTextDocument, Notification,
};
#[allow(clippy::wildcard_imports)]
use tower_lsp::lsp_types::*;
use tower_lsp::Client;
use tower_lsp::{jsonrpc::Result, lsp_types::notification::DidOpenTextDocument};

pub(crate) struct Backend {
    can_change_configuration: OnceLock<bool>,
    can_lookup_configuration: OnceLock<bool>,
    can_publish_diagnostics: OnceLock<bool>,
    client: Client,
    documents: RwLock<TextDocuments>,
    document_settings: RwLock<HashMap<Url, IdeSettings>>,
    global_settings: RwLock<IdeSettings>,
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

    async fn get_document_settings(&self, uri: &Url) -> Result<IdeSettings> {
        if !self.can_lookup_configuration.get().unwrap_or(&false) {
            self.client
                .log_message(
                    MessageType::INFO,
                    "no per-document settings lookup capability, returning global settings ...",
                )
                .await;
            let global_settings = self.global_settings.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read global settings: {e:?}"
                ))
            })?;
            return Ok(global_settings.clone());
        }

        {
            self.client
                .log_message(
                    MessageType::INFO,
                    "checking per-document settings cache ...",
                )
                .await;
            let document_settings = self.document_settings.read().map_err(|e| {
                map_err_to_internal_error(&e, format!("cannot read per-document settings: {e:?}"))
            })?;
            if let Some(settings) = document_settings.get(uri) {
                return Ok(settings.clone());
            }
        }

        self.client
            .log_message(
                MessageType::INFO,
                "fetching per-document settings for cache ...",
            )
            .await;
        let values = self
            .client
            .configuration(vec![ConfigurationItem {
                scope_uri: Some(uri.clone()),
                section: Some(String::from("nushellLanguageServer")),
            }])
            .await?;
        if let Some(value) = values.into_iter().next() {
            let settings: IdeSettings = serde_json::from_value(value).unwrap_or_default();
            let mut document_settings = self.document_settings.write().map_err(|e| {
                map_err_to_internal_error(&e, format!("cannot write per-document settings: {e:?}"))
            })?;
            document_settings.insert(uri.clone(), settings.clone());
            return Ok(settings);
        }

        self.client
            .log_message(MessageType::INFO, "fallback, returning default settings")
            .await;
        Ok(IdeSettings::default())
    }

    pub fn new(client: Client) -> Self {
        Self {
            can_change_configuration: OnceLock::new(),
            can_lookup_configuration: OnceLock::new(),
            can_publish_diagnostics: OnceLock::new(),
            client,
            documents: RwLock::new(TextDocuments::new()),
            document_settings: RwLock::new(HashMap::new()),
            global_settings: RwLock::new(IdeSettings::default()),
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

    async fn try_did_change_configuration(
        &self,
        params: DidChangeConfigurationParams,
    ) -> Result<()> {
        if *self.can_lookup_configuration.get().unwrap_or(&false) {
            let mut document_settings = self.document_settings.write().map_err(|e| {
                map_err_to_internal_error(&e, format!("cannot write per-document settings: {e:?}"))
            })?;
            document_settings.clear();
        } else {
            let settings: ClientSettingsPayload =
                serde_json::from_value(params.settings).unwrap_or_default();
            let mut global_settings = self.global_settings.write().map_err(|e| {
                map_err_to_internal_error(&e, format!("cannot write global settings: {e:?}"))
            })?;
            *global_settings = settings.nushell_language_server;
        }

        // Revalidate all open text documents
        let uris: Vec<Url> = {
            let documents = self.documents.read().map_err(|e| {
                tower_lsp::jsonrpc::Error::invalid_params(format!(
                    "cannot read from document cache: {e:?}"
                ))
            })?;
            documents.documents().keys().cloned().collect()
        };
        for uri in uris {
            self.validate_document(&uri).await?;
        }

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
        let can_publish_diagnostics = self.can_publish_diagnostics.get().unwrap_or(&false);
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

        let ide_settings = self.get_document_settings(uri).await?;
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

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct ClientSettingsPayload {
    nushell_language_server: IdeSettings,
}
