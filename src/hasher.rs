use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tree_sitter::{Node, Tree};

/// A fragment of code identified by its AST subtree.
#[derive(Debug, Clone)]
pub struct Fragment {
    pub file: PathBuf,
    pub start_line: usize,
    pub end_line: usize,
    pub node_count: usize,
    pub hash: u64,
    /// The kind of the root node (e.g. "function_item", "if_expression").
    pub kind: String,
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

/// Count nodes in a subtree.
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
fn structural_hash(node: Node, source: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();

    if is_identifier(node.kind()) {
        "ID".hash(&mut hasher);
        return hasher.finish();
    }

    if is_literal(node.kind()) {
        "LIT".hash(&mut hasher);
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
    let mut fragments = Vec::new();
    let source_bytes = source.as_bytes();
    collect_fragments_recursive(
        tree.root_node(),
        source_bytes,
        file,
        min_nodes,
        &mut fragments,
    );
    fragments
}

fn collect_fragments_recursive(
    node: Node,
    source: &[u8],
    file: &Path,
    min_nodes: usize,
    fragments: &mut Vec<Fragment>,
) {
    if is_meaningful_root(node.kind()) {
        let node_count = count_nodes(node);
        if node_count >= min_nodes {
            let hash = structural_hash(node, source);
            fragments.push(Fragment {
                file: file.to_path_buf(),
                start_line: node.start_position().row + 1,
                end_line: node.end_position().row + 1,
                node_count,
                hash,
                kind: node.kind().to_string(),
            });
        }
    }

    // Always recurse into children to find nested meaningful nodes.
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            collect_fragments_recursive(cursor.node(), source, file, min_nodes, fragments);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser;
    use std::fs;

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
        assert_eq!(fn_a.hash, fn_b.hash, "identical functions should have same hash");
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
        fs::write(
            &path_b,
            "fn bar() { let x = 1; let y = 2; let z = 3; }",
        )
        .unwrap();

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
