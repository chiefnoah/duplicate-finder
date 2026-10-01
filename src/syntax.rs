use anyhow::{anyhow, Result};
use std::ops::Range;
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

const DEFAULT: &str = "\x1b[39m";
const COMMENT: &str = "\x1b[90m";
const KEYWORD: &str = "\x1b[95m";
const STRING: &str = "\x1b[93m";
const NUMBER: &str = "\x1b[96m";
const FUNCTION: &str = "\x1b[94m";
const TYPE: &str = "\x1b[92m";
const VARIABLE: &str = "\x1b[97m";
const THEME: [(&str, &str); 12] = [
    ("attribute", NUMBER),
    ("comment", COMMENT),
    ("constant", NUMBER),
    ("function", FUNCTION),
    ("keyword", KEYWORD),
    ("number", NUMBER),
    ("operator", DEFAULT),
    ("property", VARIABLE),
    ("punctuation", DEFAULT),
    ("string", STRING),
    ("type", TYPE),
    ("variable", VARIABLE),
];

pub struct Span {
    pub bytes: Range<usize>,
    pub color: &'static str,
}

// Isolate grammar queries and highlight events from the report service.
pub fn spans(source: &str, extension: &str) -> Result<Vec<Span>> {
    let language = crate::parser::language_for_extension(extension)
        .ok_or_else(|| anyhow!("Unsupported syntax: {extension}"))?;
    if matches!(extension, "kt" | "kts") {
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&language)?;
        let tree = parser
            .parse(source, None)
            .ok_or_else(|| anyhow!("Cannot parse Kotlin syntax"))?;
        let mut spans = Vec::new();
        kotlin_spans(tree.root_node(), &mut spans);
        return Ok(spans);
    }
    let query = match extension {
        "rs" => tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
        "py" => tree_sitter_python::HIGHLIGHTS_QUERY.into(),
        "js" | "mjs" | "cjs" | "jsx" => format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
        ),
        "ts" => format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        ),
        "tsx" => format!(
            "{}\n{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        ),
        "go" => tree_sitter_go::HIGHLIGHTS_QUERY.into(),
        "java" => tree_sitter_java::HIGHLIGHTS_QUERY.into(),
        "c" | "h" => tree_sitter_c::HIGHLIGHT_QUERY.into(),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => format!(
            "{}\n{}",
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY
        ),
        "scala" | "sc" => tree_sitter_scala::HIGHLIGHTS_QUERY.into(),
        _ => return Ok(Vec::new()),
    };
    let mut config = HighlightConfiguration::new(language, extension, &query, "", "")?;
    config.configure(&THEME.map(|(name, _)| name));
    let mut highlighter = Highlighter::new();
    let mut stack = Vec::new();
    let mut spans = Vec::new();
    for event in highlighter.highlight(&config, source.as_bytes(), None, |_| None)? {
        match event? {
            HighlightEvent::HighlightStart(highlight) => stack.push(highlight.0),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let color = stack.last().map(|&index| THEME[index].1).unwrap_or(DEFAULT);
                spans.push(Span {
                    bytes: start..end,
                    color,
                });
            }
        }
    }
    Ok(spans)
}

// The Kotlin crate has no exported highlight queries; use basic AST categories.
fn kotlin_spans(node: tree_sitter::Node, spans: &mut Vec<Span>) {
    let kind = node.kind();
    let color = if kind.contains("comment") {
        COMMENT
    } else if kind.contains("string") || kind.contains("character_literal") {
        STRING
    } else if kind.contains("integer") || kind.contains("float") || kind == "number_literal" {
        NUMBER
    } else if matches!(
        kind,
        "fun"
            | "val"
            | "var"
            | "class"
            | "object"
            | "if"
            | "else"
            | "when"
            | "return"
            | "for"
            | "while"
            | "import"
            | "package"
            | "true"
            | "false"
            | "null"
    ) {
        KEYWORD
    } else if kind.contains("identifier") {
        VARIABLE
    } else {
        DEFAULT
    };
    if node.child_count() == 0 || matches!(color, COMMENT | STRING) {
        spans.push(Span {
            bytes: node.byte_range(),
            color,
        });
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        kotlin_spans(child, spans);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queries_cover_languages() {
        for (extension, source) in [
            ("rs", "fn f() { let x = 1; }"),
            ("py", "def f():\n    return 1\n"),
            ("js", "function f() { return 1; }"),
            ("ts", "function f(x: number) { return x; }"),
            ("tsx", "function f() { return <div/>; }"),
            ("go", "package main\nfunc f() {}"),
            ("java", "class A { int f() { return 1; } }"),
            ("c", "int f() { return 1; }"),
            ("cpp", "int f() { return 1; }"),
            ("kt", "fun f() { return 1 }"),
            ("scala", "object A { def f(): Int = 1 }"),
        ] {
            let result =
                spans(source, extension).unwrap_or_else(|error| panic!("{extension}: {error}"));
            assert!(
                result.iter().any(|span| span.color != DEFAULT),
                "{extension}"
            );
            assert!(result
                .iter()
                .all(|span| source.get(span.bytes.clone()).is_some()));
        }
    }
}
