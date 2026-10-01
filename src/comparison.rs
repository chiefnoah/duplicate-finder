use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;
use std::path::PathBuf;

use crate::{hasher::Fragment, similarity, syntax};

const RESET: &str = "\x1b[0m";
const MATCHED: &str = "\x1b[1m\x1b[48;5;22m";
const DEFAULT: &str = "\x1b[39m";

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Palette {
    Plain,
    Color,
}

pub struct Renderer<'a> {
    trees: &'a HashMap<PathBuf, (tree_sitter::Tree, String)>,
    styles: HashMap<PathBuf, Vec<syntax::Span>>,
    palette: Palette,
}

impl<'a> Renderer<'a> {
    pub fn new(trees: &'a HashMap<PathBuf, (tree_sitter::Tree, String)>, palette: Palette) -> Self {
        Self {
            trees,
            styles: HashMap::new(),
            palette,
        }
    }

    pub fn pair(&mut self, left: &Fragment, right: &Fragment) -> Result<String> {
        let pairs = similarity::matched_ranges(left, right, self.trees)
            .ok_or_else(|| anyhow!("Cannot align fragment source"))?;
        let (a, b): (Vec<_>, Vec<_>) = pairs.into_iter().unzip();
        let mut result = format!(
            "--- {}:{}-{}\n+++ {}:{}-{}\n",
            left.file.display(),
            left.start_line,
            left.end_line,
            right.file.display(),
            right.start_line,
            right.end_line
        );
        result.push_str(&self.fragment(left, &a)?);
        result.push_str(&self.fragment(right, &b)?);
        Ok(result)
    }

    fn fragment(&mut self, fragment: &Fragment, matched: &[Range<usize>]) -> Result<String> {
        let (_, source) = self
            .trees
            .get(&fragment.file)
            .ok_or_else(|| anyhow!("Missing source: {}", fragment.file.display()))?;
        let text = source
            .get(fragment.bytes.clone())
            .ok_or_else(|| anyhow!("Invalid fragment byte range"))?;
        let styles = if self.palette == Palette::Color {
            self.styles
                .entry(fragment.file.clone())
                .or_insert_with(|| {
                    let extension = fragment
                        .file
                        .extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or("");
                    syntax::spans(source, extension).unwrap_or_else(|error| {
                        eprintln!("warning: syntax colors unavailable: {error}");
                        Vec::new()
                    })
                })
                .as_slice()
        } else {
            &[]
        };
        Ok(render_text(
            text,
            fragment.bytes.start,
            fragment.start_line,
            matched,
            styles,
            self.palette,
        ))
    }
}

fn render_text(
    text: &str,
    start: usize,
    line: usize,
    matched: &[Range<usize>],
    styles: &[syntax::Span],
    palette: Palette,
) -> String {
    let mut output = String::new();
    let mut offset = start;
    let mut match_index = 0;
    let mut style_index = 0;
    for (index, text) in text.split_inclusive('\n').enumerate() {
        let mut content = String::new();
        let mut matched_count = 0;
        let mut unmatched_count = 0;
        let mut previous = None;
        for (relative, character) in text.char_indices() {
            let position = offset + relative;
            while match_index < matched.len() && matched[match_index].end <= position {
                match_index += 1;
            }
            while style_index < styles.len() && styles[style_index].bytes.end <= position {
                style_index += 1;
            }
            let similar = matched
                .get(match_index)
                .is_some_and(|range| range.contains(&position));
            let color = styles
                .get(style_index)
                .filter(|span| span.bytes.contains(&position))
                .map(|span| span.color)
                .unwrap_or(DEFAULT);
            if !character.is_whitespace() {
                if similar {
                    matched_count += 1;
                } else {
                    unmatched_count += 1;
                }
            }
            if palette == Palette::Color && previous != Some((color, similar)) && character != '\n'
            {
                content.push_str(RESET);
                content.push_str(color);
                if similar {
                    content.push_str(MATCHED);
                }
                previous = Some((color, similar));
            }
            // Source control characters must not inject terminal commands.
            if character.is_control() && !matches!(character, '\n' | '\t') {
                content.extend(character.escape_default());
            } else {
                content.push(character);
            }
        }
        if palette == Palette::Color {
            content.push_str(RESET);
        }
        let marker = if unmatched_count == 0 {
            '='
        } else if matched_count > 0 {
            '~'
        } else {
            '!'
        };
        let _ = write!(output, "{marker} {:>4} | {content}", line + index);
        if !text.ends_with('\n') {
            output.push('\n');
        }
        offset += text.len();
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_and_color_layers() {
        let text = "let α = 1;\n";
        let range = 0..text.len();
        let ranges = std::slice::from_ref(&range);
        let spans = vec![syntax::Span {
            bytes: 0..text.len(),
            color: "\x1b[95m",
        }];
        let plain = render_text(text, 0, 1, ranges, &spans, Palette::Plain);
        assert!(plain.contains("=    1 | let α = 1;"));
        assert!(!plain.contains('\x1b'));
        let colored = render_text(text, 0, 1, ranges, &spans, Palette::Color);
        assert!(colored.contains("\x1b[95m"));
        assert!(colored.contains(MATCHED));
        assert!(colored.contains('α'));
    }

    #[test]
    fn escape_source_controls() {
        let output = render_text("\x1b[31m", 0, 1, &[], &[], Palette::Plain);
        assert!(!output.contains('\x1b'));
    }
}
