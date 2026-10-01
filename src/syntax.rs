use anyhow::{anyhow, Result};
use std::collections::{hash_map::Entry, HashMap};
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
const OVERLAY_DEFAULT: &str = "\x1b[38;5;235m";
const OVERLAY_COMMENT: &str = "\x1b[38;5;240m";
const OVERLAY_KEYWORD: &str = "\x1b[38;5;90m";
const OVERLAY_STRING: &str = "\x1b[38;5;58m";
const OVERLAY_NUMBER: &str = "\x1b[38;5;23m";
const OVERLAY_FUNCTION: &str = "\x1b[38;5;25m";
const OVERLAY_TYPE: &str = "\x1b[38;5;22m";
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

#[derive(Default)]
pub struct Cache {
    configs: HashMap<Grammar, HighlightConfiguration>,
    highlighter: Option<Highlighter>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Grammar {
    Rust,
    Python,
    JavaScript,
    TypeScript,
    Tsx,
    Go,
    Java,
    C,
    Cpp,
    Scala,
    Kotlin,
}

impl Cache {
    // Compile once per grammar. Keep parser state local to the current report.
    pub fn spans(&mut self, source: &str, extension: &str) -> Result<Vec<Span>> {
        let key = grammar(extension).ok_or_else(|| anyhow!("Unsupported syntax: {extension}"))?;
        if key == Grammar::Kotlin {
            return kotlin_source(source, extension);
        }

        let config = match self.configs.entry(key) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(configuration(extension, key)?),
        };
        let highlighter = self.highlighter.get_or_insert_with(Highlighter::new);
        highlight_spans(highlighter, config, source)
    }
}

fn grammar(extension: &str) -> Option<Grammar> {
    match extension {
        "rs" => Some(Grammar::Rust),
        "py" => Some(Grammar::Python),
        "js" | "mjs" | "cjs" | "jsx" => Some(Grammar::JavaScript),
        "ts" => Some(Grammar::TypeScript),
        "tsx" => Some(Grammar::Tsx),
        "go" => Some(Grammar::Go),
        "java" => Some(Grammar::Java),
        "c" | "h" => Some(Grammar::C),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some(Grammar::Cpp),
        "scala" | "sc" => Some(Grammar::Scala),
        "kt" | "kts" => Some(Grammar::Kotlin),
        _ => None,
    }
}

// Darker foregrounds retain syntax hues on the pale similarity background.
pub fn overlay_color(color: &'static str) -> &'static str {
    match color {
        COMMENT => OVERLAY_COMMENT,
        KEYWORD => OVERLAY_KEYWORD,
        STRING => OVERLAY_STRING,
        NUMBER => OVERLAY_NUMBER,
        FUNCTION => OVERLAY_FUNCTION,
        TYPE => OVERLAY_TYPE,
        _ => OVERLAY_DEFAULT,
    }
}

// Isolate grammar queries and highlight events from the report service.
#[cfg(test)]
pub fn spans(source: &str, extension: &str) -> Result<Vec<Span>> {
    Cache::default().spans(source, extension)
}

fn kotlin_source(source: &str, extension: &str) -> Result<Vec<Span>> {
    let language = crate::parser::language_for_extension(extension)
        .ok_or_else(|| anyhow!("Unsupported syntax: {extension}"))?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language)?;
    let tree = parser
        .parse(source, None)
        .ok_or_else(|| anyhow!("Cannot parse Kotlin syntax"))?;
    let mut spans = Vec::new();
    kotlin_spans(tree.root_node(), &mut spans);
    Ok(spans)
}

fn configuration(extension: &str, grammar: Grammar) -> Result<HighlightConfiguration> {
    let language = crate::parser::language_for_extension(extension)
        .ok_or_else(|| anyhow!("Unsupported syntax: {extension}"))?;
    let query = match grammar {
        Grammar::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY.into(),
        Grammar::Python => tree_sitter_python::HIGHLIGHTS_QUERY.into(),
        Grammar::JavaScript => format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
        ),
        Grammar::TypeScript => format!(
            "{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        ),
        Grammar::Tsx => format!(
            "{}\n{}\n{}",
            tree_sitter_javascript::HIGHLIGHT_QUERY,
            tree_sitter_javascript::JSX_HIGHLIGHT_QUERY,
            tree_sitter_typescript::HIGHLIGHTS_QUERY
        ),
        Grammar::Go => tree_sitter_go::HIGHLIGHTS_QUERY.into(),
        Grammar::Java => tree_sitter_java::HIGHLIGHTS_QUERY.into(),
        Grammar::C => tree_sitter_c::HIGHLIGHT_QUERY.into(),
        Grammar::Cpp => format!(
            "{}\n{}",
            tree_sitter_c::HIGHLIGHT_QUERY,
            tree_sitter_cpp::HIGHLIGHT_QUERY
        ),
        Grammar::Scala => tree_sitter_scala::HIGHLIGHTS_QUERY.into(),
        Grammar::Kotlin => return Err(anyhow!("Unsupported syntax: {extension}")),
    };
    let mut config = HighlightConfiguration::new(language, extension, &query, "", "")?;
    config.configure(&THEME.map(|(name, _)| name));
    Ok(config)
}

fn highlight_spans(
    highlighter: &mut Highlighter,
    config: &HighlightConfiguration,
    source: &str,
) -> Result<Vec<Span>> {
    let mut stack = Vec::new();
    let mut spans = Vec::new();
    for event in highlighter.highlight(config, source.as_bytes(), None, |_| None)? {
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

    fn signature(spans: Vec<Span>) -> Vec<(Range<usize>, &'static str)> {
        spans
            .into_iter()
            .map(|span| (span.bytes, span.color))
            .collect()
    }

    #[test]
    fn cache_matches_fresh_output() {
        let cases = [
            ("rs", "fn f() { let α = 1; }"),
            ("py", "def f():\n    return f'{a + b}'\n"),
            ("js", "function f() { return `${a + b}`; }"),
            ("jsx", "function f() { return <div>{x}</div>; }"),
            ("ts", "function f(x: number) { return x; }"),
            ("tsx", "function f(x: string) { return <div>{x}</div>; }"),
            ("go", "package main\nfunc f() {}"),
            ("java", "class A { int f() { return 1; } }"),
            ("c", "int f() { return 1; }"),
            ("cpp", "int f() { return 1; }"),
            ("kt", "fun f() { return 1 }"),
            ("scala", "object A { def f(): Int = 1 }"),
            ("rs", "// different file\nfn g() { let β = \"text\"; }"),
            ("rs", "fn broken( {"),
            ("rs", ""),
        ];
        let mut cache = Cache::default();
        for (extension, source) in cases.into_iter().chain(cases.into_iter().rev()) {
            assert_eq!(
                signature(cache.spans(source, extension).unwrap()),
                signature(spans(source, extension).unwrap()),
                "{extension}: {source}"
            );
        }
        assert_eq!(cache.configs.len(), 10);
        assert!(cache.highlighter.is_some());
    }

    #[test]
    fn aliases_share_configurations() {
        let mut cache = Cache::default();
        for (extensions, source) in [
            (
                vec!["jsx", "mjs", "cjs", "js"],
                "function f() { return <div/>; }",
            ),
            (vec!["h", "c"], "int f() { return 1; }"),
            (
                vec!["hpp", "cc", "cxx", "hxx", "hh", "cpp"],
                "int f() { return 1; }",
            ),
            (vec!["sc", "scala"], "object A { def f(): Int = 1 }"),
        ] {
            let count = cache.configs.len();
            for extension in extensions {
                assert_eq!(
                    signature(cache.spans(source, extension).unwrap()),
                    signature(spans(source, extension).unwrap()),
                    "{extension}"
                );
            }
            assert_eq!(cache.configs.len(), count + 1);
        }
    }

    #[test]
    fn kotlin_keeps_the_fallback() {
        let mut cache = Cache::default();
        for extension in ["kt", "kts"] {
            let source = "fun f() { val α = \"text\"; return 1 }";
            assert_eq!(
                signature(cache.spans(source, extension).unwrap()),
                signature(spans(source, extension).unwrap())
            );
        }
        assert!(cache.configs.is_empty());
        assert!(cache.highlighter.is_none());
    }

    #[test]
    fn cache_is_lazy_and_recovers() {
        let mut cache = Cache::default();
        assert!(cache.configs.is_empty());
        assert!(cache.highlighter.is_none());
        for _ in 0..2 {
            assert_eq!(
                cache
                    .spans("anything", "unknown")
                    .err()
                    .unwrap()
                    .to_string(),
                "Unsupported syntax: unknown"
            );
        }
        assert!(cache.configs.is_empty());
        assert!(cache.highlighter.is_none());
        assert!(!cache.spans("fn f() {}", "rs").unwrap().is_empty());
        assert_eq!(cache.configs.len(), 1);
        assert!(cache.spans("anything", "unknown").is_err());
        assert_eq!(cache.configs.len(), 1);
        assert_eq!(
            signature(cache.spans("fn g() {}", "rs").unwrap()),
            signature(spans("fn g() {}", "rs").unwrap())
        );
    }

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
