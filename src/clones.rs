use std::collections::HashMap;

use crate::hasher::Fragment;

/// A group of code clones that share the same structural hash.
#[derive(Debug, Clone)]
pub struct CloneGroup {
    pub fragments: Vec<Fragment>,
    pub node_count: usize,
    pub similarity: f64,
}

/// Bucket fragments by hash, returning groups with 2+ members.
/// Removes subsumed clones (a clone whose line range is entirely
/// contained within a larger clone in the same group or file).
pub fn find_clone_groups(fragments: Vec<Fragment>) -> Vec<CloneGroup> {
    // Bucket by hash.
    let mut buckets: HashMap<u64, Vec<Fragment>> = HashMap::new();
    for frag in fragments {
        buckets.entry(frag.hash).or_default().push(frag);
    }

    // Keep only buckets with 2+ fragments.
    let mut groups: Vec<CloneGroup> = buckets
        .into_values()
        .filter(|frags| frags.len() >= 2)
        .map(|frags| {
            let node_count = frags[0].node_count;
            CloneGroup {
                fragments: frags,
                node_count,
                similarity: 1.0, // Exact structural match (Type 1/2)
            }
        })
        .collect();

    // Remove subsumed clones within each group.
    for group in &mut groups {
        remove_subsumed(&mut group.fragments);
    }

    // Filter out groups that collapsed to <2 after subsumption removal.
    groups.retain(|g| g.fragments.len() >= 2);

    // Sort: largest clone groups first.
    groups.sort_by(|a, b| b.node_count.cmp(&a.node_count));

    // Remove groups whose fragments are all subsumed by a larger group's fragments.
    remove_subsumed_groups(&mut groups);

    groups
}

/// Remove fragments whose line range is entirely contained within
/// another fragment in the same file.
fn remove_subsumed(fragments: &mut Vec<Fragment>) {
    // Build a list of (file, start, end) ranges from all fragments.
    let ranges: Vec<_> = fragments
        .iter()
        .map(|f| (f.file.clone(), f.start_line, f.end_line))
        .collect();

    fragments.retain(|frag| {
        // Keep this fragment unless some other fragment in the same file
        // strictly contains it (larger range).
        !ranges.iter().any(|(file, start, end)| {
            *file == frag.file
                && *start <= frag.start_line
                && *end >= frag.end_line
                && (*start < frag.start_line || *end > frag.end_line)
        })
    });
}

/// Remove groups where every fragment is contained within some fragment
/// of a larger group. Groups must be sorted largest-first.
fn remove_subsumed_groups(groups: &mut Vec<CloneGroup>) {
    let mut keep = vec![true; groups.len()];

    for i in 1..groups.len() {
        // Check if every fragment in groups[i] is subsumed by some fragment
        // in any larger group (j < i, since sorted largest-first).
        let all_subsumed = groups[i].fragments.iter().all(|frag| {
            groups[..i].iter().enumerate().any(|(j, larger)| {
                keep[j]
                    && larger.fragments.iter().any(|lf| {
                        lf.file == frag.file
                            && lf.start_line <= frag.start_line
                            && lf.end_line >= frag.end_line
                    })
            })
        });
        if all_subsumed {
            keep[i] = false;
        }
    }

    let mut idx = 0;
    groups.retain(|_| {
        let k = keep[idx];
        idx += 1;
        k
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn make_fragment(file: &str, start: usize, end: usize, nodes: usize, hash: u64) -> Fragment {
        Fragment {
            file: PathBuf::from(file),
            start_line: start,
            end_line: end,
            node_count: nodes,
            hash,
            kind: "function_item".to_string(),
        }
    }

    #[test]
    fn test_basic_clone_grouping() {
        let frags = vec![
            make_fragment("a.rs", 1, 10, 20, 111),
            make_fragment("b.rs", 5, 15, 20, 111),
            make_fragment("c.rs", 1, 5, 10, 222), // unique hash, no clone
        ];

        let groups = find_clone_groups(frags);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].fragments.len(), 2);
        assert_eq!(groups[0].similarity, 1.0);
    }

    #[test]
    fn test_subsumed_clone_removed() {
        // Fragment at lines 1-20 in a.rs subsumes fragment at lines 5-10 in a.rs.
        let frags = vec![
            make_fragment("a.rs", 1, 20, 30, 111),
            make_fragment("a.rs", 5, 10, 10, 111),
            make_fragment("b.rs", 1, 20, 30, 111),
        ];

        let groups = find_clone_groups(frags);
        assert_eq!(groups.len(), 1);
        // The subsumed fragment (5-10) should be removed, leaving a.rs:1-20 and b.rs:1-20.
        assert_eq!(groups[0].fragments.len(), 2);
        assert!(groups[0].fragments.iter().all(|f| f.start_line == 1));
    }

    #[test]
    fn test_no_clones_returns_empty() {
        let frags = vec![
            make_fragment("a.rs", 1, 10, 20, 111),
            make_fragment("b.rs", 1, 10, 20, 222),
            make_fragment("c.rs", 1, 10, 20, 333),
        ];

        let groups = find_clone_groups(frags);
        assert!(groups.is_empty());
    }

    #[test]
    fn test_groups_sorted_by_size() {
        let frags = vec![
            make_fragment("a.rs", 1, 5, 5, 111),
            make_fragment("b.rs", 1, 5, 5, 111),
            make_fragment("c.rs", 1, 20, 30, 222),
            make_fragment("d.rs", 1, 20, 30, 222),
        ];

        let groups = find_clone_groups(frags);
        assert_eq!(groups.len(), 2);
        assert!(groups[0].node_count >= groups[1].node_count);
    }

    #[test]
    fn test_cross_group_subsumption() {
        // Group with hash 111 covers a.rs:1-20 and b.rs:1-20 (large).
        // Group with hash 222 covers a.rs:5-10 and b.rs:5-10 (subsumed).
        let frags = vec![
            make_fragment("a.rs", 1, 20, 30, 111),
            make_fragment("b.rs", 1, 20, 30, 111),
            make_fragment("a.rs", 5, 10, 10, 222),
            make_fragment("b.rs", 5, 10, 10, 222),
        ];

        let groups = find_clone_groups(frags);
        assert_eq!(groups.len(), 1, "smaller group should be subsumed by larger");
        assert_eq!(groups[0].node_count, 30);
    }
}
