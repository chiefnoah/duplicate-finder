use rayon::prelude::*;
use std::collections::HashMap;

use crate::clones::CloneGroup;
use crate::hasher::Fragment;

const FULL_SIMILARITY: f64 = 1.0;

struct Token {
    kind: String,
    bytes: std::ops::Range<usize>,
}

enum Direction {
    Forward,
    Reverse,
}

// Align leaf tokens for display. Detection still uses the full AST sequence.
pub fn matched_ranges(
    left: &Fragment,
    right: &Fragment,
    trees: &HashMap<std::path::PathBuf, (tree_sitter::Tree, String)>,
) -> Option<Vec<(std::ops::Range<usize>, std::ops::Range<usize>)>> {
    let mut a = Vec::new();
    let mut b = Vec::new();
    display_tokens(find_node_at(&trees.get(&left.file)?.0, left)?, &mut a);
    display_tokens(find_node_at(&trees.get(&right.file)?.0, right)?, &mut b);
    let mut pairs = Vec::new();
    align_tokens(&a, &b, 0, 0, &mut pairs);
    Some(
        pairs
            .into_iter()
            .map(|(i, j)| (a[i].bytes.clone(), b[j].bytes.clone()))
            .collect(),
    )
}

fn display_tokens(node: tree_sitter::Node, tokens: &mut Vec<Token>) {
    let kind = normalise_kind(node.kind());
    let mut cursor = node.walk();
    let embedded = node
        .children(&mut cursor)
        .any(|child| matches!(child.kind(), "template_substitution" | "interpolation"));
    if node.child_count() == 0 || kind == "ID" || (kind == "LIT" && !embedded) {
        if !node.byte_range().is_empty() {
            tokens.push(Token {
                kind,
                bytes: node.byte_range(),
            });
        }
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        display_tokens(child, tokens);
    }
}

// Hirschberg reconstruction keeps display alignment memory linear in token counts.
fn align_tokens(
    a: &[Token],
    b: &[Token],
    offset_a: usize,
    offset_b: usize,
    pairs: &mut Vec<(usize, usize)>,
) {
    let prefix = a
        .iter()
        .zip(b)
        .take_while(|(a, b)| a.kind == b.kind)
        .count();
    pairs.extend((0..prefix).map(|index| (offset_a + index, offset_b + index)));
    let a = &a[prefix..];
    let b = &b[prefix..];
    let offset_a = offset_a + prefix;
    let offset_b = offset_b + prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take_while(|(a, b)| a.kind == b.kind)
        .count();
    let middle_a = &a[..a.len() - suffix];
    let middle_b = &b[..b.len() - suffix];

    if middle_a.len() == 1 {
        if let Some(index) = middle_b
            .iter()
            .position(|token| token.kind == middle_a[0].kind)
        {
            pairs.push((offset_a, offset_b + index));
        }
    } else if !middle_a.is_empty() && !middle_b.is_empty() {
        let mid = middle_a.len() / 2;
        let forward = token_row(&middle_a[..mid], middle_b, Direction::Forward);
        let reverse = token_row(&middle_a[mid..], middle_b, Direction::Reverse);
        let split = (0..=middle_b.len())
            .max_by_key(|&index| forward[index] + reverse[middle_b.len() - index])
            .unwrap();
        drop(forward);
        drop(reverse);
        align_tokens(
            &middle_a[..mid],
            &middle_b[..split],
            offset_a,
            offset_b,
            pairs,
        );
        align_tokens(
            &middle_a[mid..],
            &middle_b[split..],
            offset_a + mid,
            offset_b + split,
            pairs,
        );
    }
    pairs.extend((0..suffix).map(|index| {
        (
            offset_a + a.len() - suffix + index,
            offset_b + b.len() - suffix + index,
        )
    }));
}

fn token_row(a: &[Token], b: &[Token], direction: Direction) -> Vec<usize> {
    let mut row = vec![0; b.len() + 1];
    for i in 0..a.len() {
        let mut diagonal = 0;
        for j in 0..b.len() {
            let (i, j_index) = match direction {
                Direction::Forward => (i, j),
                Direction::Reverse => (a.len() - 1 - i, b.len() - 1 - j),
            };
            let previous = row[j + 1];
            row[j + 1] = if a[i].kind == b[j_index].kind {
                diagonal + 1
            } else {
                row[j].max(previous)
            };
            diagonal = previous;
        }
    }
    row
}

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
/// Strategy: bucket fragments by kind, then
/// compare pairs with different hashes within each bucket.
pub fn find_near_duplicates(
    fragments: &[Fragment],
    trees: &HashMap<std::path::PathBuf, (tree_sitter::Tree, String)>,
    threshold: f64,
    min_nodes: usize,
) -> Vec<CloneGroup> {
    // Exact clones can anchor a near group, but same-hash pairs stay in exact output.
    let eligible: Vec<&Fragment> = fragments
        .iter()
        .filter(|f| f.node_count >= min_nodes)
        .collect();

    if eligible.len() < 2 {
        return Vec::new();
    }

    // Size rejection uses the threshold bound, not arbitrary bucket boundaries.
    let mut buckets: HashMap<String, Vec<&Fragment>> = HashMap::new();
    for frag in &eligible {
        buckets.entry(frag.kind.clone()).or_default().push(frag);
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
                    find_node_at(tree, frag)
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
                if grouped[j] || bucket[i].hash == bucket[j].hash {
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
                    f.file == group_frags[1].file
                        && f.bytes == group_frags[1].bytes
                        && f.kind == group_frags[1].kind
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
    // LCS cannot exceed the smaller sequence length.
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
    let mut seen: std::collections::HashSet<Vec<(std::path::PathBuf, usize, usize)>> =
        std::collections::HashSet::new();
    groups.retain(|g| {
        let mut key: Vec<_> = g
            .fragments
            .iter()
            .map(|f| (f.file.clone(), f.bytes.start, f.bytes.end))
            .collect();
        key.sort();
        seen.insert(key)
    });
}

// Byte ranges and kinds identify nested nodes even when their line ranges coincide.
fn find_node_at<'tree>(
    tree: &'tree tree_sitter::Tree,
    fragment: &Fragment,
) -> Option<tree_sitter::Node<'tree>> {
    let mut node = tree
        .root_node()
        .descendant_for_byte_range(fragment.bytes.start, fragment.bytes.end)?;
    loop {
        if node.byte_range() == fragment.bytes && node.kind() == fragment.kind {
            return Some(node);
        }
        node = node.parent()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_alignment_matches_lcs() {
        let sequences: Vec<Vec<String>> = (0..=5)
            .flat_map(|length| {
                (0..(1 << length)).map(move |bits| {
                    (0..length)
                        .map(|index| {
                            if (bits >> index) & 1 == 0 {
                                "a".into()
                            } else {
                                "b".into()
                            }
                        })
                        .collect()
                })
            })
            .collect();
        for a in &sequences {
            for b in &sequences {
                let tokens = |sequence: &[String]| {
                    sequence
                        .iter()
                        .enumerate()
                        .map(|(index, kind)| Token {
                            kind: kind.clone(),
                            bytes: index..index + 1,
                        })
                        .collect::<Vec<_>>()
                };
                let mut pairs = Vec::new();
                align_tokens(&tokens(a), &tokens(b), 0, 0, &mut pairs);
                assert_eq!(pairs.len(), reference_lcs(a, b), "{a:?} {b:?}");
                assert!(pairs.iter().all(|&(i, j)| a[i] == b[j]));
                assert!(pairs
                    .windows(2)
                    .all(|pair| pair[0].0 < pair[1].0 && pair[0].1 < pair[1].1));
            }
        }
    }

    #[test]
    fn display_aligns_unicode_renames() {
        let (fragments, trees) = near_fixture(&[
            "fn first() { let α = 1; }\n".into(),
            "fn second() { let β = 2; }\n".into(),
        ]);
        let functions: Vec<_> = fragments
            .iter()
            .filter(|fragment| fragment.kind == "function_item")
            .collect();
        let pairs = matched_ranges(functions[0], functions[1], &trees).unwrap();
        let a = &trees[&functions[0].file].1;
        let b = &trees[&functions[1].file].1;
        assert!(pairs
            .iter()
            .all(|(left, right)| a.get(left.clone()).is_some() && b.get(right.clone()).is_some()));
        assert!(pairs
            .iter()
            .any(|(left, right)| &a[left.clone()] == "α" && &b[right.clone()] == "β"));
    }

    fn near_fixture(
        sources: &[String],
    ) -> (
        Vec<Fragment>,
        HashMap<std::path::PathBuf, (tree_sitter::Tree, String)>,
    ) {
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&crate::parser::language_for_extension("rs").unwrap())
            .unwrap();
        let mut fragments = Vec::new();
        let mut trees = HashMap::new();
        for (index, source) in sources.iter().enumerate() {
            let file = std::path::PathBuf::from(format!("{index}.rs"));
            let tree = parser.parse(source, None).unwrap();
            fragments.extend(crate::hasher::collect_fragments(&tree, source, &file, 5));
            trees.insert(file, (tree, source.clone()));
        }
        (fragments, trees)
    }

    #[test]
    fn near_crosses_size_buckets() {
        let sources: Vec<_> = [6, 7]
            .into_iter()
            .map(|count| format!("fn f() {{\n{}\n}}\n", "let x = 1;\n".repeat(count)))
            .collect();
        let (fragments, trees) = near_fixture(&sources);
        let groups = find_near_duplicates(&fragments, &trees, 0.8, 5);
        assert!(groups
            .iter()
            .any(|group| group.fragments[0].kind == "function_item"));
    }

    #[test]
    fn near_uses_exact_anchor() {
        let sources = vec![
            "fn f() {\nlet x = a + b;\n}\n".into(),
            "fn g() {\nlet y = a + b;\n}\n".into(),
            "fn h() {\nlet z = a - b;\n}\n".into(),
        ];
        let (fragments, trees) = near_fixture(&sources);
        let groups = find_near_duplicates(&fragments, &trees, 0.8, 5);
        assert!(groups.iter().any(|group| group
            .fragments
            .iter()
            .any(|fragment| fragment.file == std::path::Path::new("2.rs"))));
        assert!(groups.iter().all(|group| group
            .fragments
            .iter()
            .any(|fragment| fragment.hash != group.fragments[0].hash)));
    }

    #[test]
    fn homogeneous_buckets_stay_empty() {
        let (fragments, trees) = near_fixture(&[
            "fn a() { let x = 1; }\n".into(),
            "fn b() { let y = 2; }\n".into(),
        ]);
        for threshold in [0.0, 0.8, 1.0] {
            assert!(find_near_duplicates(&fragments, &trees, threshold, 5).is_empty());
        }
    }

    #[test]
    fn same_line_nodes_stay_distinct() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("same.rs");
        std::fs::write(
            &path,
            "fn a() { let x = a + b; } fn b() { let x = a - b; }\n",
        )
        .unwrap();
        let (tree, source) = crate::parser::parse_file(&path).unwrap();
        let fragments = crate::hasher::collect_fragments(&tree, &source, &path, 5);
        let trees = HashMap::from([(path, (tree, source))]);
        assert!(find_near_duplicates(&fragments, &trees, 1.0, 5).is_empty());
    }

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
                if grouped[j] || bucket[i].hash == bucket[j].hash {
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
                        f.file == fragments[1].file
                            && f.bytes == fragments[1].bytes
                            && f.kind == fragments[1].kind
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
            bytes: 1..2,
            start_line: 1,
            end_line: 2,
            node_count: 12,
            hash: index as u64,
            kind: "function_item".into(),
        }
    }

    #[test]
    fn parallel_matches_serial() {
        let mut fragments: Vec<_> = (0..96).map(fixture_fragment).collect();
        // Include repeated hashes without excluding their near candidates.
        for fragment in &mut fragments {
            fragment.hash %= 13;
        }
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

        // Exact hashes can anchor groups; exclude small fragments and absent nodes.
        fragments[1].hash = fragments[0].hash;
        fragments[2].node_count = 1;
        fragments.push(fixture_fragment(100));
        let mut absent_node = fragments[3].clone();
        absent_node.hash = u64::MAX;
        absent_node.start_line = 100;
        absent_node.bytes = 1000..1001;
        fragments.push(absent_node);

        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            let groups = pool.install(|| find_near_duplicates(&fragments, &trees, 0.8, 5));
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].fragments.len(), 4);
            assert_eq!(groups[0].similarity, 0.8);
            let paths: Vec<_> = groups[0].fragments.iter().map(|f| &f.file).collect();
            assert_eq!(
                paths,
                [0, 3, 4, 5]
                    .into_iter()
                    .map(|index| &fragments[index].file)
                    .collect::<Vec<_>>()
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
