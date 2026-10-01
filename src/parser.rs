use anyhow::{anyhow, Result};
use std::path::Path;
use tree_sitter::Language;

pub fn language_for_extension(ext: &str) -> Option<Language> {
    match ext {
        "rs" => Some(tree_sitter_rust::LANGUAGE.into()),
        "py" => Some(tree_sitter_python::LANGUAGE.into()),
        "js" | "mjs" | "cjs" | "jsx" => Some(tree_sitter_javascript::LANGUAGE.into()),
        "ts" => Some(tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()),
        "tsx" => Some(tree_sitter_typescript::LANGUAGE_TSX.into()),
        "go" => Some(tree_sitter_go::LANGUAGE.into()),
        "java" => Some(tree_sitter_java::LANGUAGE.into()),
        "c" | "h" => Some(tree_sitter_c::LANGUAGE.into()),
        "cpp" | "cc" | "cxx" | "hpp" | "hxx" | "hh" => Some(tree_sitter_cpp::LANGUAGE.into()),
        "kt" | "kts" => Some(tree_sitter_kotlin_ng::LANGUAGE.into()),
        "scala" | "sc" => Some(tree_sitter_scala::LANGUAGE.into()),
        _ => None,
    }
}

pub fn parse_file(path: &Path) -> Result<(tree_sitter::Tree, String)> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| anyhow!("no extension: {}", path.display()))?;

    let language =
        language_for_extension(ext).ok_or_else(|| anyhow!("unsupported language: .{ext}"))?;

    let source = std::fs::read_to_string(path)
        .map_err(|e| anyhow!("failed to read {}: {e}", path.display()))?;

    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language)?;

    let tree = parser
        .parse(&source, None)
        .ok_or_else(|| anyhow!("parse failed: {}", path.display()))?;

    Ok((tree, source))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parse_typed_tsx() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("card.tsx");
        fs::write(
            &file,
            "export function Card(props: {name: string}) { return <div>{props.name}</div>; }",
        )
        .unwrap();
        let (tree, _) = parse_file(&file).unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_language_for_known_extensions() {
        assert!(language_for_extension("rs").is_some());
        assert!(language_for_extension("py").is_some());
        assert!(language_for_extension("js").is_some());
        assert!(language_for_extension("ts").is_some());
        assert!(language_for_extension("go").is_some());
        assert!(language_for_extension("java").is_some());
        assert!(language_for_extension("c").is_some());
        assert!(language_for_extension("cpp").is_some());
    }

    #[test]
    fn test_language_for_kotlin() {
        assert!(language_for_extension("kt").is_some());
        assert!(language_for_extension("kts").is_some());
    }

    #[test]
    fn test_language_for_scala() {
        assert!(language_for_extension("scala").is_some());
        assert!(language_for_extension("sc").is_some());
    }

    #[test]
    fn test_parse_scala_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.scala");
        fs::write(
            &path,
            "object Main {\n  def hello(): Unit = println(\"hi\")\n}\n",
        )
        .unwrap();

        let (tree, _source) = parse_file(&path).unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_language_for_unknown_extension() {
        assert!(language_for_extension("txt").is_none());
        assert!(language_for_extension("md").is_none());
    }

    #[test]
    fn test_parse_rust_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rs");
        fs::write(&path, "fn main() { let x = 42; }").unwrap();

        let (tree, source) = parse_file(&path).unwrap();
        assert!(!tree.root_node().has_error());
        assert_eq!(source, "fn main() { let x = 42; }");
    }

    #[test]
    fn test_parse_python_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.py");
        fs::write(&path, "def hello():\n    print('hi')\n").unwrap();

        let (tree, _source) = parse_file(&path).unwrap();
        assert!(!tree.root_node().has_error());
    }

    #[test]
    fn test_parse_unsupported_extension() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        fs::write(&path, "hello").unwrap();

        assert!(parse_file(&path).is_err());
    }
}
