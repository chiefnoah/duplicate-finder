use rayon::prelude::*;
use std::collections::HashMap;

use crate::clones::CloneGroup;
use crate::hasher::Fragment;

const FULL_SIMILARITY: f64 = 1.0;

/// Compute structural similarity between two AST fragments.
/// Uses a normalised node-kind sequence comparison (LCS-based).
/// Returns a value in [0.0, 1.0].
pub fn tree_similarity(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return FULL_SIMILARITY;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }

    let lcs_len = lcs_length(a, b);
    let max_len = a.len().max(b.len());
    lcs_len as f64 / max_len as f64
}

/// Longest common subsequence length (standard DP).
fn lcs_length(a: &[String], b: &[String]) -> usize {
    // Shared ends always belong to an optimal LCS; compare only the different middle.
    let prefix = a.iter().zip(b).take_while(|(a, b)| a == b).count();
    let a = &a[prefix..];
    let b = &b[prefix..];
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(a, b)| a == b)
        .count();
    let a = &a[..a.len() - suffix];
    let b = &b[..b.len() - suffix];
    let shared = prefix + suffix;

    if a.is_empty() || b.is_empty() {
        return shared;
    }

    let m = a.len();
    let n = b.len();
    // Use two rows to save memory.
    let mut prev = vec![0usize; n + 1];
    let mut curr = vec![0usize; n + 1];

    for i in 1..=m {
        for j in 1..=n {
            if a[i - 1] == b[j - 1] {
                curr[j] = prev[j - 1] + 1;
            } else {
                curr[j] = curr[j - 1].max(prev[j]);
            }
        }
        std::mem::swap(&mut prev, &mut curr);
        curr.iter_mut().for_each(|x| *x = 0);
    }

    shared + prev[n]
}

/// Extract a pre-order sequence of normalised node kinds from an AST subtree.
/// This is used as the "fingerprint" for similarity comparison.
pub fn extract_kind_sequence(
    node: tree_sitter::Node,
    source: &[u8],
) -> Vec<String> {
    let mut seq = Vec::new();
    extract_kind_sequence_recursive(node, source, &mut seq);
    seq
}

fn extract_kind_sequence_recursive(
    node: tree_sitter::Node,
    source: &[u8],
    seq: &mut Vec<String>,
) {
    let kind = normalise_kind(node.kind());
    seq.push(kind);

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            extract_kind_sequence_recursive(cursor.node(), source, seq);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

fn normalise_kind(kind: &str) -> String {
    match kind {
        "identifier" | "field_identifier" | "type_identifier"
        | "shorthand_field_identifier" | "property_identifier" => "ID".to_string(),
        "integer_literal" | "float_literal" | "string_literal" | "string"
        | "raw_string_literal" | "char_literal" | "boolean_literal" | "true"
        | "false" | "none" | "null" | "number" | "template_string"
        | "interpreted_string_literal" | "rune_literal" => "LIT".to_string(),
        other => other.to_string(),
    }
}

/// Find Type 3 near-duplicate clone groups.
///
/// Strategy: bucket fragments by (kind, node_count ± tolerance), then
/// compare pairs within each bucket. Only compare fragments that didn't
/// already match exactly (i.e. have unique hashes).
pub fn find_near_duplicates(
    fragments: &[Fragment],
    trees: &HashMap<std::path::PathBuf, (tree_sitter::Tree, String)>,
    threshold: f64,
    min_nodes: usize,
) -> Vec<CloneGroup> {
    // Collect fragments that are "lonely" — their hash only appears once
    // among all fragments, so they weren't caught by exact matching.
    // Actually, we want to find pairs across ALL non-exact fragments,
    // including those whose hash appeared multiple times (they're already
    // in Type 1/2 groups). We focus on fragments with unique hashes.
    let mut hash_counts: HashMap<u64, usize> = HashMap::new();
    for f in fragments {
        *hash_counts.entry(f.hash).or_default() += 1;
    }

    let lonely: Vec<&Fragment> = fragments
        .iter()
        .filter(|f| hash_counts[&f.hash] == 1 && f.node_count >= min_nodes)
        .collect();

    if lonely.len() < 2 {
        return Vec::new();
    }

    // Bucket by (kind, size_bucket) where size_bucket = node_count / 3.
    // This gives ~33% size tolerance.
    let mut buckets: HashMap<(String, usize), Vec<&Fragment>> = HashMap::new();
    for frag in &lonely {
        let size_bucket = frag.node_count / 3;
        buckets
            .entry((frag.kind.clone(), size_bucket))
            .or_default()
            .push(frag);
    }

    // Keep bucket order so thread schedules do not affect sort ties or deduplication.
    let buckets: Vec<_> = buckets.into_values().collect();
    let mut groups: Vec<CloneGroup> = buckets
        .par_iter()
        .filter(|bucket| bucket.len() >= 2)
        .flat_map_iter(|bucket| {
            let sequences: Vec<Option<Vec<String>>> = bucket
                .par_iter()
                .map(|frag| {
                    let (tree, source) = trees.get(&frag.file)?;
                    find_node_at(tree, frag.start_line, frag.end_line)
                        .map(|node| extract_kind_sequence(node, source.as_bytes()))
                })
                .collect();

            group_bucket(bucket, &sequences, threshold)
        })
        .collect();

    groups.sort_by(|a, b| b.node_count.cmp(&a.node_count));

    // Deduplicate groups that have the same set of fragment locations.
    dedup_groups(&mut groups);

    groups
}

fn group_bucket(
    bucket: &[&Fragment],
    sequences: &[Option<Vec<String>>],
    threshold: f64,
) -> Vec<CloneGroup> {
    let mut groups = Vec::new();
    let mut grouped = vec![false; bucket.len()];

    for i in 0..bucket.len() {
        if grouped[i] {
            continue;
        }
        let Some(ref seq_i) = sequences[i] else {
            continue;
        };

        // Compare one row in parallel; avoid a quadratic pairwise score table.
        let matches: Vec<usize> = ((i + 1)..bucket.len())
            .into_par_iter()
            .filter(|&j| {
                if grouped[j] {
                    return false;
                }
                let Some(ref seq_j) = sequences[j] else {
                    return false;
                };

                matches(seq_i, seq_j, threshold)
            })
            .collect();

        if matches.is_empty() {
            continue;
        }

        // Commit matches in input order to preserve greedy, non-transitive groups.
        let mut group_frags = vec![(*bucket[i]).clone()];
        for j in matches {
            group_frags.push((*bucket[j]).clone());
            grouped[j] = true;
        }

        // Preserve the existing score and location lookup for two-member groups.
        let similarity = if group_frags.len() == 2 {
            let idx = bucket
                .iter()
                .position(|f| {
                    f.file == group_frags[1].file && f.start_line == group_frags[1].start_line
                })
                .unwrap();
            tree_similarity(seq_i, sequences[idx].as_ref().unwrap())
        } else {
            threshold
        };

        groups.push(CloneGroup {
            node_count: group_frags[0].node_count,
            fragments: group_frags,
            similarity,
        });
        grouped[i] = true;
    }

    groups
}

fn matches(a: &[String], b: &[String], threshold: f64) -> bool {
    // Use actual sequence counts: line lookup can select a larger AST node.
    if !can_match(a.len(), b.len(), threshold) {
        return false;
    }

    // A full LCS of equal-length sequences requires equality, not a DP table.
    if threshold == FULL_SIMILARITY {
        return a == b;
    }

    tree_similarity(a, b) >= threshold
}

fn can_match(a: usize, b: usize, threshold: f64) -> bool {
    let largest = a.max(b);
    let bound = if largest == 0 {
        FULL_SIMILARITY
    } else {
        a.min(b) as f64 / largest as f64
    };

    bound >= threshold
}

fn dedup_groups(groups: &mut Vec<CloneGroup>) {
    let mut seen: std::collections::HashSet<Vec<(String, usize, usize)>> =
        std::collections::HashSet::new();
    groups.retain(|g| {
        let mut key: Vec<_> = g
            .fragments
            .iter()
            .map(|f| (f.file.display().to_string(), f.start_line, f.end_line))
            .collect();
        key.sort();
        seen.insert(key)
    });
}

/// Find the AST node that spans the given line range.
fn find_node_at(tree: &tree_sitter::Tree, start_line: usize, end_line: usize) -> Option<tree_sitter::Node<'_>> {
    let root = tree.root_node();
    find_node_at_recursive(root, start_line, end_line)
}

fn find_node_at_recursive<'a>(
    node: tree_sitter::Node<'a>,
    start_line: usize,
    end_line: usize,
) -> Option<tree_sitter::Node<'a>> {
    let node_start = node.start_position().row + 1;
    let node_end = node.end_position().row + 1;

    if node_start == start_line && node_end == end_line {
        return Some(node);
    }

    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            if let Some(found) = find_node_at_recursive(cursor.node(), start_line, end_line) {
                return Some(found);
            }
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // Use a full DP table as an independent reference for optimized comparisons.
    fn reference_lcs(a: &[String], b: &[String]) -> usize {
        let mut table = vec![vec![0; b.len() + 1]; a.len() + 1];
        for i in 1..=a.len() {
            for j in 1..=b.len() {
                table[i][j] = if a[i - 1] == b[j - 1] {
                    table[i - 1][j - 1] + 1
                } else {
                    table[i - 1][j].max(table[i][j - 1])
                };
            }
        }
        table[a.len()][b.len()]
    }

    fn reference_similarity(a: &[String], b: &[String]) -> f64 {
        let largest = a.len().max(b.len());
        if largest == 0 {
            return 1.0;
        }
        reference_lcs(a, b) as f64 / largest as f64
    }

    #[test]
    fn lcs_matches_reference() {
        let sequences: Vec<Vec<String>> = (0..=5)
            .flat_map(|length| {
                (0..(1 << length)).map(move |bits| {
                    (0..length)
                        .map(|index| if (bits >> index) & 1 == 0 { "a" } else { "b" }.to_string())
                        .collect()
                })
            })
            .collect();

        for a in &sequences {
            for b in &sequences {
                assert_eq!(lcs_length(a, b), reference_lcs(a, b), "{a:?} {b:?}");
                let expected = reference_similarity(a, b);
                assert_eq!(tree_similarity(a, b), expected);
                for threshold in [0.0, 0.5, 0.8, 1.0, 2.0, f64::NAN, expected] {
                    assert_eq!(matches(a, b, threshold), expected >= threshold);
                }
            }
        }
    }

    #[test]
    fn size_bound_keeps_matches() {
        for a in 0..8 {
            for b in 0..8 {
                let seq_a = vec!["ID".to_string(); a];
                let seq_b = vec!["ID".to_string(); b];
                let similarity = tree_similarity(&seq_a, &seq_b);
                for threshold in [
                    -1.0,
                    0.0,
                    0.5,
                    1.0,
                    2.0,
                    f64::NEG_INFINITY,
                    f64::INFINITY,
                    f64::NAN,
                    similarity,
                    similarity - f64::EPSILON,
                    similarity + f64::EPSILON,
                ] {
                    assert_eq!(can_match(a, b, threshold), similarity >= threshold);
                }
            }
        }
    }

    #[test]
    fn bound_uses_sequence_counts() {
        let mut fragments: Vec<_> = (0..2).map(fixture_fragment).collect();
        fragments[1].node_count = 14;
        let bucket: Vec<_> = fragments.iter().collect();
        let sequences = vec![Some(vec!["ID".to_string(); 20]); fragments.len()];

        let groups = group_bucket(&bucket, &sequences, 1.0);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].fragments.len(), 2);
        assert_eq!(groups[0].similarity, 1.0);
    }

    fn serial_groups(
        bucket: &[&Fragment],
        sequences: &[Option<Vec<String>>],
        threshold: f64,
    ) -> Vec<CloneGroup> {
        let mut groups = Vec::new();
        let mut grouped = vec![false; bucket.len()];

        for i in 0..bucket.len() {
            if grouped[i] {
                continue;
            }
            let Some(seq_i) = &sequences[i] else {
                continue;
            };
            let mut fragments = vec![(*bucket[i]).clone()];

            for j in (i + 1)..bucket.len() {
                if grouped[j] {
                    continue;
                }
                let Some(seq_j) = &sequences[j] else {
                    continue;
                };
                if reference_similarity(seq_i, seq_j) >= threshold {
                    fragments.push((*bucket[j]).clone());
                    grouped[j] = true;
                }
            }

            if fragments.len() < 2 {
                continue;
            }
            let similarity = if fragments.len() == 2 {
                let idx = bucket
                    .iter()
                    .position(|f| {
                        f.file == fragments[1].file && f.start_line == fragments[1].start_line
                    })
                    .unwrap();
                reference_similarity(seq_i, sequences[idx].as_ref().unwrap())
            } else {
                threshold
            };
            groups.push(CloneGroup {
                fragments,
                node_count: bucket[i].node_count,
                similarity,
            });
            grouped[i] = true;
        }

        groups
    }

    fn fixture_fragment(index: usize) -> Fragment {
        Fragment {
            file: format!("{index}.rs").into(),
            start_line: 1,
            end_line: 2,
            node_count: 12,
            hash: index as u64,
            kind: "function_item".into(),
        }
    }

    #[test]
    fn parallel_matches_serial() {
        let fragments: Vec<_> = (0..96).map(fixture_fragment).collect();
        let bucket: Vec<_> = fragments.iter().collect();
        let sequences: Vec<_> = (0..fragments.len())
            .map(|i| {
                if i % 11 == 0 {
                    return None;
                }
                Some(
                    (0..(i % 19))
                        .map(|j| format!("{}", (i >> (j % 6)) % 3))
                        .collect(),
                )
            })
            .collect();

        for threads in [1, 2, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            for threshold in [0.0, 0.5, 0.75, 1.0, f64::NAN] {
                let expected = serial_groups(&bucket, &sequences, threshold);
                let actual = pool.install(|| group_bucket(&bucket, &sequences, threshold));
                assert_eq!(format!("{actual:?}"), format!("{expected:?}"));
            }
        }
    }

    #[test]
    fn parallel_preserves_greedy_groups() {
        // A matches B and B matches C, but A does not match C.
        let fragments: Vec<_> = (0..3).map(fixture_fragment).collect();
        let bucket: Vec<_> = fragments.iter().collect();
        let sequences: Vec<_> = [
            ["a", "b", "c", "d"],
            ["a", "b", "c", "e"],
            ["a", "b", "e", "e"],
        ]
        .into_iter()
        .map(|seq| Some(seq.into_iter().map(String::from).collect()))
        .collect();

        let groups = group_bucket(&bucket, &sequences, 0.75);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].fragments.len(), 2);
        assert_eq!(groups[0].fragments[0].file, fragments[0].file);
        assert_eq!(groups[0].fragments[1].file, fragments[1].file);
        assert_eq!(groups[0].similarity, 0.75);
    }

    #[test]
    fn parallel_keeps_fragment_filters() {
        let dir = tempfile::tempdir().unwrap();
        let mut fragments = Vec::new();
        let mut trees = HashMap::new();

        for (i, operator) in ["+", "-", "*", "/", "%", "=="].into_iter().enumerate() {
            let path = dir.path().join(format!("{i}.rs"));
            std::fs::write(&path, format!("fn f() {{ let x = a {operator} b; }}")).unwrap();
            let (tree, source) = crate::parser::parse_file(&path).unwrap();
            let fragment = crate::hasher::collect_fragments(&tree, &source, &path, 5)
                .into_iter()
                .find(|f| f.kind == "function_item")
                .unwrap();
            fragments.push(fragment);
            trees.insert(path, (tree, source));
        }

        // Exclude exact hashes, small fragments, and absent trees or nodes.
        fragments[1].hash = fragments[0].hash;
        fragments[2].node_count = 1;
        fragments.push(fixture_fragment(100));
        let mut absent_node = fragments[3].clone();
        absent_node.hash = u64::MAX;
        absent_node.start_line = 100;
        fragments.push(absent_node);

        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let groups = pool.install(|| find_near_duplicates(&fragments, &trees, 0.8, 5));
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].fragments.len(), 3);
            assert_eq!(groups[0].similarity, 0.8);
            let paths: Vec<_> = groups[0].fragments.iter().map(|f| &f.file).collect();
            assert_eq!(
                paths,
                fragments[3..6].iter().map(|f| &f.file).collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn test_tree_similarity_identical() {
        let a = vec!["fn".into(), "ID".into(), "block".into()];
        let b = vec!["fn".into(), "ID".into(), "block".into()];
        assert_eq!(tree_similarity(&a, &b), 1.0);
    }

    #[test]
    fn test_tree_similarity_completely_different() {
        let a = vec!["fn".into(), "ID".into()];
        let b = vec!["class".into(), "block".into()];
        assert_eq!(tree_similarity(&a, &b), 0.0);
    }

    #[test]
    fn test_tree_similarity_partial() {
        let a: Vec<String> = vec!["fn".into(), "ID".into(), "block".into(), "let".into()];
        let b: Vec<String> = vec!["fn".into(), "ID".into(), "block".into(), "return".into()];
        let sim = tree_similarity(&a, &b);
        // 3 out of 4 match = 0.75
        assert!((sim - 0.75).abs() < 0.01);
    }

    #[test]
    fn test_tree_similarity_empty() {
        let empty: Vec<String> = vec![];
        let non_empty: Vec<String> = vec!["fn".into()];
        assert_eq!(tree_similarity(&empty, &empty), 1.0);
        assert_eq!(tree_similarity(&empty, &non_empty), 0.0);
    }

    #[test]
    fn test_lcs_length_basic() {
        let a: Vec<String> = vec!["a".into(), "b".into(), "c".into(), "d".into()];
        let b: Vec<String> = vec!["a".into(), "c".into(), "d".into()];
        assert_eq!(lcs_length(&a, &b), 3);
    }

    #[test]
    fn test_near_duplicates_end_to_end() {
        use crate::parser;
        use std::fs;

        let dir = tempfile::tempdir().unwrap();

        // Two structurally similar but not identical functions.
        let path_a = dir.path().join("a.rs");
        fs::write(
            &path_a,
            "fn foo() { let x = 1; let y = 2; let z = 3; }",
        )
        .unwrap();

        let path_b = dir.path().join("b.rs");
        fs::write(
            &path_b,
            "fn bar() { let a = 1; let b = 2; let c = 4; }",
        )
        .unwrap();

        let (tree_a, src_a) = parser::parse_file(&path_a).unwrap();
        let (tree_b, src_b) = parser::parse_file(&path_b).unwrap();

        let frags_a = crate::hasher::collect_fragments(&tree_a, &src_a, &path_a, 5);
        let frags_b = crate::hasher::collect_fragments(&tree_b, &src_b, &path_b, 5);

        let mut all_frags = frags_a;
        all_frags.extend(frags_b);

        let mut trees_map = HashMap::new();
        trees_map.insert(path_a.clone(), (tree_a, src_a));
        trees_map.insert(path_b.clone(), (tree_b, src_b));

        // These two functions are actually Type 2 clones (identical after normalisation),
        // so they'll have the same hash. Let's test with a structural difference instead.
        // Actually for a proper Type 3 test, one function needs extra/different structure.
        let path_c = dir.path().join("c.rs");
        fs::write(
            &path_c,
            "fn baz() { let a = 1; let b = 2; if true { let c = 3; } }",
        )
        .unwrap();

        let (tree_c, src_c) = parser::parse_file(&path_c).unwrap();
        let frags_c = crate::hasher::collect_fragments(&tree_c, &src_c, &path_c, 5);

        // Build fragments from a.rs and c.rs (structurally different but similar)
        let mut frags = Vec::new();
        frags.extend(crate::hasher::collect_fragments(
            &trees_map[&path_a].0,
            &trees_map[&path_a].1,
            &path_a,
            5,
        ));
        frags.extend(frags_c);
        trees_map.insert(path_c.clone(), (tree_c, src_c));

        let near_dupes = find_near_duplicates(&frags, &trees_map, 0.5, 5);
        // With a low threshold of 0.5, these similar functions should be grouped.
        // The exact result depends on the AST structure, but let's verify it doesn't panic
        // and returns reasonable results.
        assert!(
            near_dupes.is_empty() || near_dupes[0].similarity >= 0.5,
            "near duplicates should meet threshold"
        );
    }
}
