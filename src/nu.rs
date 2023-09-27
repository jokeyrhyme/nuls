use std::path::PathBuf;

use serde::Deserialize;
use tower_lsp::lsp_types::Position;

#[derive(Deserialize)]
pub(crate) struct IdeComplete {
    pub completions: Vec<String>,
}

#[derive(Deserialize)]
pub(crate) struct IdeGotoDef {
    // end: usize,
    pub file: PathBuf,
    // start: usize,
}

#[derive(Deserialize)]
pub(crate) struct IdeHover {
    pub hover: String,
    // span: Option<Range>,
}

// ported from https://github.com/nushell/vscode-nushell-lang
pub(crate) fn convert_position(position: &Position, text: &str) -> usize {
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
#[allow(dead_code)]
pub(crate) fn convert_span() -> Position {
    todo!()
}

// ported from https://github.com/nushell/vscode-nushell-lang
#[allow(dead_code)]
pub(crate) fn find_line_breaks(text: &str) -> Vec<usize> {
    text.as_bytes()
        .iter()
        .enumerate()
        .filter_map(|(i, b)| if b == &0x0a { Some(i) } else { None })
        .collect()
}

// ported from https://github.com/nushell/vscode-nushell-lang
#[allow(dead_code)]
pub(crate) fn lower_bound_binary_search(_offset: usize, _line_breaks: &[usize]) -> Position {
    Position::default()
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
        assert_eq!(convert_position(&Position::default(), FIXTURE.trim()), 0);

        // `ls | ...`
        assert_eq!(
            convert_position(
                &Position {
                    line: 2,
                    character: 4
                },
                FIXTURE.trim()
            ),
            37
        );
    }

    #[test]
    fn find_line_breaks_ok() {
        assert_eq!(find_line_breaks(FIXTURE.trim()), vec![18, 32, 64]);
    }
}
