use std::{cmp::max, ffi::OsStr, path::PathBuf, time::Duration};

use serde::Deserialize;
use tokio::fs;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{Position, Url};

use crate::error::{map_err_to_internal_error, map_err_to_parse_error};

#[derive(Deserialize)]
pub(crate) struct IdeComplete {
    pub completions: Vec<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
pub(crate) struct IdeGotoDef {
    pub end: usize,
    pub file: PathBuf,
    pub start: usize,
}

#[derive(Deserialize)]
pub(crate) struct IdeHover {
    pub hover: String,
    pub span: Option<IdeHoverSpan>,
}
#[derive(Deserialize)]
pub(crate) struct IdeHoverSpan {
    pub end: usize,
    pub start: usize,
}

#[allow(dead_code)]
#[derive(Clone, Debug)]
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
#[derive(Clone, Debug)]
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

// ported from https://github.com/nushell/vscode-nushell-lang
pub(crate) fn convert_position(position: Position, text: &str) -> usize {
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

// ported from https://github.com/nushell/vscode-nushell-lang
pub(crate) fn convert_span(offset: usize, line_breaks: &[usize]) -> Position {
    let line_break_index = lower_bound_binary_search(line_breaks, offset);

    match line_break_index {
        Some(i) => {
            let start_of_line_offset = line_breaks[i] + 1;
            let character = max(0, offset - start_of_line_offset);

            Position {
                line: u32::try_from(i + 1).unwrap_or_default(),
                character: u32::try_from(character).unwrap_or_default(),
            }
        }
        None => Position::default(),
    }
}

// ported from https://github.com/nushell/vscode-nushell-lang
pub(crate) fn find_line_breaks(text: &str) -> Vec<usize> {
    text.as_bytes()
        .iter()
        .enumerate()
        .filter_map(|(i, b)| if b == &0x0a { Some(i) } else { None })
        .collect()
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
    uri: Url,
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

// ported from https://github.com/nushell/vscode-nushell-lang
/// returns the index of the line-break prior to the byte offset
fn lower_bound_binary_search(line_breaks: &[usize], offset: usize) -> Option<usize> {
    if line_breaks.is_empty() {
        return None;
    }

    let mut low = 0;
    let mut mid: usize;
    let mut high = line_breaks.len() - 1;

    if offset >= line_breaks[high] {
        return Some(high);
    };

    while low < high {
        // Bitshift to avoid floating point division
        mid = (low + high) >> 1;

        if line_breaks[mid] < offset {
            low = mid + 1;
        } else {
            high = mid;
        }
    }

    if low > 0 {
        Some(low - 1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static FIXTURE: &str = "
#! /usr/bin/env nu
def main [] {
    ls | sort-by 'size' | first
}
";

    #[test]
    fn convert_position_ok() {
        assert_eq!(convert_position(Position::default(), FIXTURE.trim()), 0);

        // `ls | ...`
        assert_eq!(
            convert_position(
                Position {
                    line: 2,
                    character: 4
                },
                FIXTURE.trim()
            ),
            37
        );
    }

    #[test]
    fn convert_span_ok() {
        let line_breaks = find_line_breaks(FIXTURE.trim());

        assert_eq!(convert_span(0, &line_breaks), Position::default());

        // `ls | ...`
        assert_eq!(
            convert_span(37, &line_breaks),
            Position {
                line: 2,
                character: 4
            }
        );
    }

    #[test]
    fn find_line_breaks_ok() {
        assert_eq!(find_line_breaks(FIXTURE.trim()), vec![18, 32, 64]);
    }

    #[test]
    fn lower_bound_binary_search_ok() {
        let line_breaks = find_line_breaks(FIXTURE.trim());

        assert_eq!(lower_bound_binary_search(&[], 0), None);
        assert_eq!(lower_bound_binary_search(&line_breaks, 15), None);
        assert_eq!(lower_bound_binary_search(&line_breaks, 30), Some(0));
        assert_eq!(lower_bound_binary_search(&line_breaks, 50), Some(1));
        assert_eq!(lower_bound_binary_search(&line_breaks, 70), Some(2));
    }
}
