use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Tree};

/// A fragment of code identified by its AST subtree.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub file: PathBuf,
    pub bytes: std::ops::Range<usize>,
    pub start_line: usize,
    pub end_line: usize,
    pub node_count: usize,
    pub hash: u64,
    /// The kind of the root node (e.g. "function_item", "if_expression").
    pub kind: String,
}

#[derive(Clone, Copy, Default)]
struct Subtree {
    hash: u64,
    node_count: usize,
}

/// Meaningful root node kinds — we only hash subtrees rooted at these.
/// This keeps the output actionable and avoids hashing every tiny expression.
fn is_meaningful_root(kind: &str) -> bool {
    matches!(
        kind,
        "function_item"
            | "function_definition"
            | "function_declaration"
            | "method_definition"
            | "method_declaration"
            | "arrow_function"
            | "lambda"
            | "if_statement"
            | "if_expression"
            | "for_statement"
            | "for_expression"
            | "for_in_statement"
            | "while_statement"
            | "while_expression"
            | "match_expression"
            | "switch_statement"
            | "switch_expression"
            | "block"
            | "expression_statement"
            | "class_declaration"
            | "class_definition"
            | "impl_item"
            | "struct_item"
            | "struct_definition"
            | "try_statement"
            | "with_statement"
            | "when_expression"
            | "object_declaration"
            | "object_definition"
            | "trait_definition"
            | "val_definition"
            | "var_definition"
    )
}

/// Node kinds that represent identifiers — normalised away for Type 2 detection.
fn is_identifier(kind: &str) -> bool {
    matches!(
        kind,
        "identifier"
            | "field_identifier"
            | "type_identifier"
            | "shorthand_field_identifier"
            | "property_identifier"
            | "simple_identifier"
    )
}

/// Node kinds that represent literals — normalised to type-only placeholders.
fn is_literal(kind: &str) -> bool {
    matches!(
        kind,
        "integer_literal"
            | "float_literal"
            | "string_literal"
            | "string"
            | "raw_string_literal"
            | "char_literal"
            | "boolean_literal"
            | "true"
            | "false"
            | "none"
            | "null"
            | "number"
            | "template_string"
            | "interpreted_string_literal"
            | "rune_literal"
            | "line_string_literal"
            | "multi_line_string_literal"
    )
}

// Literal text is replaceable; executable substitutions are not.
fn is_interpolation(kind: &str) -> bool {
    matches!(kind, "template_substitution" | "interpolation")
}

// Retain the former count and hash traversals as test-only output oracles.
/// Count nodes in a subtree.
#[cfg(test)]
fn count_nodes(node: Node) -> usize {
    let mut count = 1;
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            count += count_nodes(cursor.node());
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    count
}

/// Compute a normalised structural hash for a subtree.
/// Identifiers are replaced with "ID", literals with "LIT".
/// The hash is a Merkle hash: hash(kind, child_hashes...).
#[cfg(test)]
fn structural_hash(node: Node, source: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();

    if is_identifier(node.kind()) {
        "ID".hash(&mut hasher);
        return hasher.finish();
    }

    if is_literal(node.kind()) {
        "LIT".hash(&mut hasher);
        let mut cursor = node.walk();
        for child in node
            .children(&mut cursor)
            .filter(|child| is_interpolation(child.kind()))
        {
            structural_hash(child, source).hash(&mut hasher);
        }
        return hasher.finish();
    }

    // For leaf nodes with no children, hash the kind + text.
    if node.child_count() == 0 {
        node.kind().hash(&mut hasher);
        let text = &source[node.byte_range()];
        text.hash(&mut hasher);
        return hasher.finish();
    }

    // Interior node: hash kind + ordered child hashes.
    node.kind().hash(&mut hasher);
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            let child_hash = structural_hash(cursor.node(), source);
            child_hash.hash(&mut hasher);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
    hasher.finish()
}

/// Walk the AST and collect fragments for all meaningful subtrees
/// above the minimum node count.
pub fn collect_fragments(
    tree: &Tree,
    source: &str,
    file: &Path,
    min_nodes: usize,
) -> Vec<Fragment> {
    let mut subtrees = Vec::new();
    collect_subtrees(tree.root_node(), source.as_bytes(), &mut subtrees);

    // Cached counts filter candidates before fragment allocation.
    subtrees
        .into_iter()
        .filter(|(_, subtree)| subtree.node_count >= min_nodes)
        .map(|(node, subtree)| Fragment {
            file: file.to_path_buf(),
            bytes: node.byte_range(),
            start_line: node.start_position().row + 1,
            end_line: node.end_position().row + 1,
            node_count: subtree.node_count,
            hash: subtree.hash,
            kind: node.kind().to_string(),
        })
        .collect()
}

fn collect_subtrees<'tree>(
    node: Node<'tree>,
    source: &[u8],
    subtrees: &mut Vec<(Node<'tree>, Subtree)>,
) -> Subtree {
    // Reserve preorder slots; fill metadata after the children to preserve group order.
    let slot = if is_meaningful_root(node.kind()) {
        let index = subtrees.len();
        subtrees.push((node, Subtree::default()));
        Some(index)
    } else {
        None
    };

    let mut hasher = DefaultHasher::new();
    let normalized = if is_identifier(node.kind()) {
        "ID".hash(&mut hasher);
        true
    } else if is_literal(node.kind()) {
        "LIT".hash(&mut hasher);
        true
    } else {
        node.kind().hash(&mut hasher);
        false
    };

    if !normalized && node.child_count() == 0 {
        source[node.byte_range()].hash(&mut hasher);
    }

    // Fold child metadata once. Normalized literals still count all descendants.
    let mut node_count = 1;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        let subtree = collect_subtrees(child, source, subtrees);
        node_count += subtree.node_count;
        if !normalized || (is_literal(node.kind()) && is_interpolation(child.kind())) {
            subtree.hash.hash(&mut hasher);
        }
    }

    let subtree = Subtree {
        hash: hasher.finish(),
        node_count,
    };
    if let Some(index) = slot {
        subtrees[index].1 = subtree;
    }

    subtree
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::fs;

    #[test]
    fn interpolation_preserves_code() {
        for (extension, first, changed, renamed) in [
            (
                "js",
                "function f() { return `${a + b}`; }",
                "function f() { return `${a * b}`; }",
                "function g() { return `${x + y}`; }",
            ),
            (
                "py",
                "def f():\n    return f'{a + b}'\n",
                "def f():\n    return f'{a * b}'\n",
                "def g():\n    return f'{x + y}'\n",
            ),
        ] {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&crate::parser::language_for_extension(extension).unwrap())
                .unwrap();
            let hashes: Vec<_> = [first, changed, renamed]
                .into_iter()
                .map(|source| {
                    let tree = parser.parse(source, None).unwrap();
                    collect_fragments(&tree, source, Path::new("fixture"), 5)[0].hash
                })
                .collect();
            assert_ne!(
                hashes[0], hashes[1],
                "{extension}: changed executable expression"
            );
            assert_eq!(hashes[0], hashes[2], "{extension}: renamed identifiers");
        }
    }

    // Preserve the original preorder traversal as an independent output oracle.
    fn reference_fragments(
        node: Node,
        source: &[u8],
        file: &Path,
        min_nodes: usize,
        fragments: &mut Vec<Fragment>,
    ) {
        let node_count = count_nodes(node);
        if is_meaningful_root(node.kind()) && node_count >= min_nodes {
            fragments.push(Fragment {
                file: file.to_path_buf(),
                bytes: node.byte_range(),
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                node_count,
                hash: structural_hash(node, source),
                kind: node.kind().into(),
            });
        }

        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            reference_fragments(child, source, file, min_nodes, fragments);
        }
    }

    #[test]
    fn fragments_match_reference() {
        let sources = [
            ("rs", "fn outer() { let s = r#\"hello\"#; if true { fn inner() {} } }"),
            ("py", "def outer():\n    def inner():\n        return 'hello'\n    if True:\n        return inner()\n"),
            ("js", "function f() { return `${(() => { return a + 1; })()}`; }"),
            ("ts", "class A { f(x: number) { if (x > 0) { return x; } return 0; } }"),
            ("go", "package main\nfunc f(x int) int { if x > 0 { return x }; return 0 }"),
            ("java", "class A { int f(int x) { if (x > 0) { return x; } return 0; } }"),
            ("c", "int f(int x) { if (x > 0) { return x; } return 0; }"),
            ("cpp", "class A { public: int f(int x) { if (x > 0) { return x; } return 0; } };"),
            ("kt", "fun f(x: Int): Int { if (x > 0) { return x }; return 0 }"),
            ("scala", "object A { def f(x: Int): Int = { if (x > 0) { x } else { 0 } } }"),
            ("rs", "fn broken() { let x = ; if true {"),
        ];

        for (extension, source) in sources {
            let mut parser = tree_sitter::Parser::new();
            parser
                .set_language(&parser::language_for_extension(extension).unwrap())
                .unwrap();
            let tree = parser.parse(source, None).unwrap();
            let file = PathBuf::from(format!("fixture.{extension}"));
            let total = count_nodes(tree.root_node());

            for min_nodes in [0, 1, 5, total, total + 1] {
                let mut expected = Vec::new();
                reference_fragments(
                    tree.root_node(),
                    source.as_bytes(),
                    &file,
                    min_nodes,
                    &mut expected,
                );
                let actual = collect_fragments(&tree, source, &file, min_nodes);
                assert_eq!(
                    format!("{actual:?}"),
                    format!("{expected:?}"),
                    "{extension}: {source}"
                );
            }
        }
    }

    #[test]
    fn test_identical_functions_same_hash() {
        let dir = tempfile::tempdir().unwrap();

        let path_a = dir.path().join("a.rs");
        fs::write(&path_a, "fn foo() { let x = 1; let y = 2; }").unwrap();

        let path_b = dir.path().join("b.rs");
        fs::write(&path_b, "fn foo() { let x = 1; let y = 2; }").unwrap();

        let (tree_a, src_a) = parser::parse_file(&path_a).unwrap();
        let (tree_b, src_b) = parser::parse_file(&path_b).unwrap();

        let frags_a = collect_fragments(&tree_a, &src_a, &path_a, 3);
        let frags_b = collect_fragments(&tree_b, &src_b, &path_b, 3);

        // Both should have at least one fragment (the function_item)
        let fn_a = frags_a.iter().find(|f| f.kind == "function_item").unwrap();
        let fn_b = frags_b.iter().find(|f| f.kind == "function_item").unwrap();
        assert_eq!(
            fn_a.hash, fn_b.hash,
            "identical functions should have same hash"
        );
    }

    #[test]
    fn test_renamed_variables_same_hash_type2() {
        let dir = tempfile::tempdir().unwrap();

        let path_a = dir.path().join("a.rs");
        fs::write(&path_a, "fn foo() { let alpha = 1; let beta = 2; }").unwrap();

        let path_b = dir.path().join("b.rs");
        fs::write(&path_b, "fn bar() { let x = 1; let y = 2; }").unwrap();

        let (tree_a, src_a) = parser::parse_file(&path_a).unwrap();
        let (tree_b, src_b) = parser::parse_file(&path_b).unwrap();

        let frags_a = collect_fragments(&tree_a, &src_a, &path_a, 3);
        let frags_b = collect_fragments(&tree_b, &src_b, &path_b, 3);

        let fn_a = frags_a.iter().find(|f| f.kind == "function_item").unwrap();
        let fn_b = frags_b.iter().find(|f| f.kind == "function_item").unwrap();
        assert_eq!(
            fn_a.hash, fn_b.hash,
            "functions with renamed identifiers should have same hash (Type 2)"
        );
    }

    #[test]
    fn test_different_structure_different_hash() {
        let dir = tempfile::tempdir().unwrap();

        let path_a = dir.path().join("a.rs");
        fs::write(&path_a, "fn foo() { let x = 1; }").unwrap();

        let path_b = dir.path().join("b.rs");
        fs::write(&path_b, "fn bar() { let x = 1; let y = 2; let z = 3; }").unwrap();

        let (tree_a, src_a) = parser::parse_file(&path_a).unwrap();
        let (tree_b, src_b) = parser::parse_file(&path_b).unwrap();

        let frags_a = collect_fragments(&tree_a, &src_a, &path_a, 3);
        let frags_b = collect_fragments(&tree_b, &src_b, &path_b, 3);

        let fn_a = frags_a.iter().find(|f| f.kind == "function_item").unwrap();
        let fn_b = frags_b.iter().find(|f| f.kind == "function_item").unwrap();
        assert_ne!(
            fn_a.hash, fn_b.hash,
            "structurally different functions should have different hashes"
        );
    }

    #[test]
    fn test_min_nodes_filters_small_subtrees() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.rs");
        fs::write(&path, "fn f() { let x = 1; }").unwrap();

        let (tree, src) = parser::parse_file(&path).unwrap();

        let frags_high = collect_fragments(&tree, &src, &path, 100);
        assert!(
            frags_high.is_empty(),
            "very high min_nodes should filter everything"
        );

        let frags_low = collect_fragments(&tree, &src, &path, 1);
        assert!(!frags_low.is_empty(), "low min_nodes should keep fragments");
    }
}
