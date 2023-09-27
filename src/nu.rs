use std::{cmp::max, path::PathBuf};

use serde::Deserialize;
use tower_lsp::lsp_types::Position;

#[derive(Deserialize)]
pub(crate) struct IdeComplete {
    pub completions: Vec<String>,
}

#[derive(Deserialize)]
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

// ported from https://github.com/nushell/vscode-nushell-lang
/// returns the index of the line_break prior to the byte offset
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
