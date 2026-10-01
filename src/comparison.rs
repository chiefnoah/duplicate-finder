use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Range;
use std::path::PathBuf;

use crate::{hasher::Fragment, similarity, syntax};

const RESET: &str = "\x1b[0m";
const MATCHED: &str = "\x1b[48;5;194m";
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
    reference: Option<similarity::Reference>,
    syntax: syntax::Cache,
}

impl<'a> Renderer<'a> {
    pub fn new(trees: &'a HashMap<PathBuf, (tree_sitter::Tree, String)>, palette: Palette) -> Self {
        Self {
            trees,
            styles: HashMap::new(),
            palette,
            reference: None,
            syntax: syntax::Cache::default(),
        }
    }

    // Bound token reuse to the current group; syntax caches remain per file.
    pub fn begin_group(&mut self) {
        self.reference = None;
    }

    pub fn pair(&mut self, left: &Fragment, right: &Fragment) -> Result<String> {
        if !self
            .reference
            .as_ref()
            .is_some_and(|reference| reference.is_fragment(left))
        {
            self.reference = Some(
                similarity::Reference::new(left, self.trees)
                    .ok_or_else(|| anyhow!("Cannot align fragment source"))?,
            );
        }
        let pairs = self
            .reference
            .as_ref()
            .unwrap()
            .ranges(right, self.trees)
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
                    self.syntax
                        .spans(source, extension)
                        .unwrap_or_else(|error| {
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
    // Skip preceding source spans without a linear walk for every fragment.
    let mut style_index = styles.partition_point(|span| span.bytes.end <= start);
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
            let color = if similar {
                syntax::overlay_color(color)
            } else {
                color
            };
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
    fn span_seek_preserves_output() {
        let spans = [
            syntax::Span {
                bytes: 0..3,
                color: "\x1b[95m",
            },
            syntax::Span {
                bytes: 4..8,
                color: "\x1b[94m",
            },
            syntax::Span {
                bytes: 8..10,
                color: "\x1b[93m",
            },
            syntax::Span {
                bytes: 11..12,
                color: DEFAULT,
            },
            syntax::Span {
                bytes: 13..15,
                color: "\x1b[97m",
            },
        ];
        for start in 0..17 {
            let matched = start..start + "α".len();
            let skipped = spans
                .iter()
                .take_while(|span| span.bytes.end <= start)
                .count();
            for palette in [Palette::Plain, Palette::Color] {
                let expected = render_text(
                    "α + β",
                    start,
                    42,
                    std::slice::from_ref(&matched),
                    &spans[skipped..],
                    palette,
                );
                assert_eq!(
                    render_text(
                        "α + β",
                        start,
                        42,
                        std::slice::from_ref(&matched),
                        &spans,
                        palette
                    ),
                    expected
                );
            }
        }
    }

    fn fixture() -> (Vec<Fragment>, HashMap<PathBuf, (tree_sitter::Tree, String)>) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&crate::parser::language_for_extension("rs").unwrap())
            .unwrap();
        let mut fragments = Vec::new();
        let mut trees = HashMap::new();
        for (file, source) in [
            ("a.rs", "fn first() { let α = a + b; }\n"),
            ("b.rs", "fn second() { let β = a - b; }\n"),
            ("c.rs", "fn third() { let γ = a + b; }\n"),
        ] {
            let tree = parser.parse(source, None).unwrap();
            let file = PathBuf::from(file);
            let fragment = crate::hasher::collect_fragments(&tree, source, &file, 5)
                .into_iter()
                .find(|fragment| fragment.kind == "function_item")
                .unwrap();
            fragments.push(fragment);
            trees.insert(file, (tree, source.into()));
        }
        (fragments, trees)
    }

    #[test]
    fn pair_output_snapshot() {
        let (fragments, trees) = fixture();
        for palette in [Palette::Plain, Palette::Color] {
            let mut renderer = Renderer::new(&trees, palette);
            let result = renderer.pair(&fragments[0], &fragments[1]).unwrap();
            let header = "--- a.rs:1-1\n+++ b.rs:1-1\n";
            let expected = match palette {
                Palette::Plain => format!("{header}~    1 | fn first() {{ let α = a + b; }}\n~    1 | fn second() {{ let β = a - b; }}\n"),
                Palette::Color => format!("{header}{}{}", color_line("first", "α", "+"), color_line("second", "β", "-")),
            };
            assert_eq!(result, expected);
            assert_eq!(result, renderer.pair(&fragments[0], &fragments[1]).unwrap());
        }
    }

    #[test]
    fn group_cache_preserves_pairs() {
        let (fragments, trees) = fixture();
        for palette in [Palette::Plain, Palette::Color] {
            let mut renderer = Renderer::new(&trees, palette);
            for (left, right) in [(0, 1), (0, 2), (2, 1), (1, 0)] {
                let mut fresh = Renderer::new(&trees, palette);
                assert_eq!(
                    renderer.pair(&fragments[left], &fragments[right]).unwrap(),
                    fresh.pair(&fragments[left], &fragments[right]).unwrap()
                );
            }
            renderer.begin_group();
            assert!(renderer.reference.is_none());
            assert_eq!(
                renderer.pair(&fragments[0], &fragments[1]).unwrap(),
                Renderer::new(&trees, palette)
                    .pair(&fragments[0], &fragments[1])
                    .unwrap()
            );

            let mut invalid = fragments[0].clone();
            invalid.bytes.start += 1;
            assert!(renderer.pair(&invalid, &fragments[1]).is_err());
            invalid = fragments[0].clone();
            invalid.kind = "missing_kind".into();
            assert!(renderer.pair(&invalid, &fragments[1]).is_err());
        }
    }

    // Fixed ANSI transitions for the pale overlay and readable syntax foregrounds.
    fn color_line(name: &str, identifier: &str, operator: &str) -> String {
        format!(concat!(
            "~    1 | \x1b[0m\x1b[38;5;90m\x1b[48;5;194mfn\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;25m\x1b[48;5;194m{}\x1b[0m\x1b[38;5;235m\x1b[48;5;194m()\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;235m\x1b[48;5;194m{{\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;90m\x1b[48;5;194mlet\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;235m\x1b[48;5;194m{}\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;235m\x1b[48;5;194m=\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;235m\x1b[48;5;194ma\x1b[0m\x1b[39m {} ",
            "\x1b[0m\x1b[38;5;235m\x1b[48;5;194mb;\x1b[0m\x1b[39m ",
            "\x1b[0m\x1b[38;5;235m\x1b[48;5;194m}}\x1b[0m\n"
        ), name, identifier, operator)
    }

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
        assert!(colored.contains("\x1b[38;5;90m"));
        assert!(colored.contains(MATCHED));
        assert!(colored.contains('α'));
    }

    #[test]
    fn pale_overlay_keeps_syntax() {
        let range = 0..2;
        let spans = [syntax::Span {
            bytes: range.clone(),
            color: "\x1b[95m",
        }];
        let output = render_text(
            "fn",
            0,
            1,
            std::slice::from_ref(&range),
            &spans,
            Palette::Color,
        );
        assert!(output.contains("\x1b[48;5;194m"));
        assert!(output.contains("\x1b[38;5;90m"));
        assert!(!output.contains("\x1b[1m"));
    }

    #[test]
    fn escape_source_controls() {
        let output = render_text("\x1b[31m", 0, 1, &[], &[], Palette::Plain);
        assert!(!output.contains('\x1b'));
    }
}
