use std::{ffi::OsStr, path::PathBuf, time::Duration};

use lsp_textdocument::FullTextDocument;
use serde::Deserialize;
use tokio::fs;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, DiagnosticSeverity, Range, Url,
};
use tower_lsp::{jsonrpc::Result, lsp_types::Diagnostic};

use crate::error::{map_err_to_internal_error, map_err_to_parse_error};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase", tag = "type")]
pub(crate) enum IdeCheck {
    Diagnostic(IdeCheckDiagnostic),
    Hint(IdeCheckHint),
}

#[derive(Debug, Deserialize, PartialEq)]
pub(crate) struct IdeCheckDiagnostic {
    pub message: String,
    pub severity: IdeDiagnosticSeverity,
    pub span: IdeSpan,
}
impl IdeCheckDiagnostic {
    pub fn to_diagnostic(&self, doc: &FullTextDocument, uri: &Url) -> Diagnostic {
        Diagnostic {
            message: self.message.clone(),
            range: Range {
                end: doc.position_at(self.span.end),
                start: doc.position_at(self.span.start),
            },
            severity: Some(DiagnosticSeverity::from(&self.severity)),
            source: Some(String::from(uri.clone())),
            ..Diagnostic::default()
        }
    }
}

#[derive(Debug, Deserialize, PartialEq)]
pub(crate) struct IdeCheckHint {
    pub position: IdeSpan,
    pub typename: String,
}

#[derive(Deserialize)]
pub(crate) struct IdeComplete {
    pub completions: Vec<String>,
}
impl TryFrom<CompilerResponse> for IdeComplete {
    type Error = tower_lsp::jsonrpc::Error;

    fn try_from(value: CompilerResponse) -> std::result::Result<Self, Self::Error> {
        serde_json::from_slice(value.stdout.as_bytes()).map_err(|e| {
            map_err_to_parse_error(e, format!("cannot parse response from {}", value.cmdline))
        })
    }
}
impl From<IdeComplete> for CompletionResponse {
    fn from(value: IdeComplete) -> Self {
        CompletionResponse::Array(
            value
                .completions
                .into_iter()
                .enumerate()
                .map(|(i, c)| {
                    let kind = if c.contains('(') {
                        CompletionItemKind::FUNCTION
                    } else {
                        CompletionItemKind::FIELD
                    };
                    CompletionItem {
                        data: Some(serde_json::Value::from(i + 1)),
                        kind: Some(kind),
                        label: c,
                        ..Default::default()
                    }
                })
                .collect(),
        )
    }
}

#[derive(Debug, Deserialize, PartialEq)]
pub(crate) enum IdeDiagnosticSeverity {
    Error,
    Warning,
    Information,
    Hint,
}
impl From<&IdeDiagnosticSeverity> for DiagnosticSeverity {
    fn from(value: &IdeDiagnosticSeverity) -> Self {
        match value {
            IdeDiagnosticSeverity::Error => Self::ERROR,
            IdeDiagnosticSeverity::Warning => Self::WARNING,
            IdeDiagnosticSeverity::Information => Self::INFORMATION,
            IdeDiagnosticSeverity::Hint => Self::HINT,
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct IdeGotoDef {
    pub end: u32,
    pub file: PathBuf,
    pub start: u32,
}

#[derive(Deserialize)]
pub(crate) struct IdeHover {
    pub hover: String,
    pub span: Option<IdeSpan>,
}
#[derive(Debug, Deserialize, PartialEq)]
pub(crate) struct IdeSpan {
    pub end: u32,
    pub start: u32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct IdeSettings {
    pub hints: IdeSettingsHints,
    pub include_dirs: Vec<PathBuf>,
    pub max_number_of_problems: u32,
    pub max_nushell_invocation_time: Duration,
    pub nushell_executable_path: PathBuf,
}
impl Default for IdeSettings {
    fn default() -> Self {
        Self {
            hints: IdeSettingsHints::default(),
            include_dirs: vec![],
            max_number_of_problems: 1000,
            max_nushell_invocation_time: Duration::from_secs(10),
            nushell_executable_path: PathBuf::from("nu"),
        }
    }
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct IdeSettingsHints {
    pub show_inferred_types: bool,
}
impl Default for IdeSettingsHints {
    fn default() -> Self {
        Self {
            show_inferred_types: true,
        }
    }
}

pub(crate) struct CompilerResponse {
    pub cmdline: String,
    pub stdout: String,
}

// ported from https://github.com/nushell/vscode-nushell-lang
pub(crate) async fn run_compiler(
    text: &str,
    mut flags: Vec<&OsStr>,
    settings: IdeSettings,
    uri: &Url,
) -> Result<CompilerResponse> {
    // TODO: support allowErrors and label options like vscode-nushell-lang?

    let max_number_of_problems = format!("{}", settings.max_number_of_problems);
    let max_number_of_problems_flag = OsStr::new(&max_number_of_problems);
    if flags.contains(&OsStr::new("--ide-check")) {
        flags.push(max_number_of_problems_flag);
    }

    // record separator character (a character that is unlikely to appear in a path)
    let record_separator: &OsStr = OsStr::new("\x1e");
    let mut include_paths: Vec<PathBuf> = vec![];
    if uri.scheme() == "file" {
        let file_path = uri.to_file_path().map_err(|e| {
            tower_lsp::jsonrpc::Error::invalid_params(format!(
                "cannot convert URI to filesystem path: {e:?}",
            ))
        })?;
        if let Some(p) = file_path.parent() {
            include_paths.push(p.to_path_buf());
        }
    }
    if !settings.include_dirs.is_empty() {
        include_paths.extend(settings.include_dirs);
    }
    let include_paths: Vec<&OsStr> = include_paths.iter().map(OsStr::new).collect();
    let include_paths_flag = include_paths.join(record_separator);
    if !include_paths.is_empty() {
        flags.push(OsStr::new("--include-path"));
        flags.push(&include_paths_flag);
    }

    // vscode-nushell-lang creates this once per single-threaded server process,
    // but we create this here to ensure the temporary file is used once-per-request
    let temp_file = mktemp::Temp::new_file().map_err(|e| {
        map_err_to_internal_error(e, String::from("unable to create temporary file"))
    })?;
    fs::write(&temp_file, text).await.map_err(|e| {
        map_err_to_internal_error(e, String::from("unable to write to temporary file"))
    })?;
    flags.push(temp_file.as_os_str());

    let cmdline = format!("nu {flags:?}");

    // TODO: honour max_nushell_invocation_time like vscode-nushell-lang
    // TODO: call settings.nushell_executable_path

    // TODO: call nushell Rust code directly instead of via separate process,
    // https://github.com/jokeyrhyme/nuls/issues/7
    let output = tokio::process::Command::new("nu")
        .args(flags)
        .output()
        .await
        .map_err(|e| map_err_to_internal_error(e, format!("`{cmdline}` failed")))?;
    let stdout = String::from_utf8(output.stdout).map_err(|e| {
        map_err_to_parse_error(e, format!("`{cmdline}` did not return valid UTF-8"))
    })?;
    Ok(CompilerResponse { cmdline, stdout })
}

#[cfg(test)]
mod tests {
    use tower_lsp::lsp_types::{DiagnosticSeverity, Position};

    use super::*;

    #[test]
    fn deserialize_ide_check_diagnostic() {
        let input = r#"{"message":"Missing required positional argument.","severity":"Error","span":{"end":1026,"start":1026},"type":"diagnostic"}"#;

        let got: IdeCheck = serde_json::from_str(input).expect("cannot deserialize");

        assert_eq!(
            got,
            IdeCheck::Diagnostic(IdeCheckDiagnostic {
                message: String::from("Missing required positional argument."),
                severity: IdeDiagnosticSeverity::Error,
                span: IdeSpan {
                    end: 1026,
                    start: 1026
                }
            })
        );
    }

    #[test]
    fn ide_check_diagnostic_to_diagnostic() {
        let input = IdeCheckDiagnostic {
            message: String::from("Missing required positional argument."),
            severity: IdeDiagnosticSeverity::Error,
            span: IdeSpan { end: 0, start: 0 },
        };
        let doc = FullTextDocument::new(String::new(), 0, String::from("foo"));
        let uri = Url::parse("file:///foo").expect("cannot parse URL");

        let got = input.to_diagnostic(&doc, &uri);

        assert_eq!(
            got,
            Diagnostic {
                message: String::from("Missing required positional argument."),
                range: Range {
                    end: Position {
                        line: 0,
                        character: 0
                    },
                    start: Position {
                        line: 0,
                        character: 0
                    },
                },
                severity: Some(DiagnosticSeverity::ERROR),
                source: Some(uri.to_string()),
                ..Diagnostic::default()
            }
        );
    }

    #[tokio::test]
    async fn completion_ok() {
        let output = run_compiler(
            "wh",
            vec![OsStr::new("--ide-complete"), OsStr::new(&format!("{}", 2))],
            IdeSettings::default(),
            &Url::parse("file:///foo.nu").expect("unable to parse test URL"),
        )
        .await
        .expect("unable to run `nu --ide-complete ...`");

        let complete = IdeComplete::try_from(output)
            .expect("unable to convert output from `nu --ide-complete ...`");
        let got = CompletionResponse::from(complete);

        if let CompletionResponse::Array(v) = &got {
            // sequence is non-deterministic,
            // so this is more reliable than using an assert_eq!() for the whole collection
            v.iter()
                .find(|c| c.label == *"where" || c.kind == Some(CompletionItemKind::FIELD))
                .expect("'where' not in list");
            v.iter()
                .find(|c| c.label == *"which" || c.kind == Some(CompletionItemKind::FIELD))
                .expect("'which' not in list");
            v.iter()
                .find(|c| c.label == *"while" || c.kind == Some(CompletionItemKind::FIELD))
                .expect("'while' not in list");
        } else {
            unreachable!();
        }
    }
}
