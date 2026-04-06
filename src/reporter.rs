use crate::clones::CloneGroup;

pub fn print_report(exact_groups: &[CloneGroup], near_groups: &[CloneGroup]) {
    let total = exact_groups.len() + near_groups.len();
    if total == 0 {
        println!("No clones found.");
        return;
    }

    println!("Found {} clone group(s)\n", total);

    let mut group_num = 1;

    for group in exact_groups {
        print_group(group, group_num);
        group_num += 1;
    }

    for group in near_groups {
        print_group(group, group_num);
        group_num += 1;
    }
}

fn print_group(group: &CloneGroup, num: usize) {
    let similarity_pct = (group.similarity * 100.0).round() as u32;
    let clone_type = if similarity_pct == 100 {
        "exact"
    } else {
        "near"
    };

    println!(
        "Clone Group {} ({} clone(s), {} nodes, {}% similarity, {})",
        num,
        group.fragments.len(),
        group.node_count,
        similarity_pct,
        clone_type,
    );

    for frag in &group.fragments {
        println!("  {}:{}-{}", frag.file.display(), frag.start_line, frag.end_line);
    }
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hasher::Fragment;
    use std::path::PathBuf;

    #[test]
    fn test_print_report_no_clones() {
        // Just ensure it doesn't panic.
        print_report(&[], &[]);
    }

    #[test]
    fn test_print_report_with_groups() {
        let group = CloneGroup {
            fragments: vec![
                Fragment {
                    file: PathBuf::from("src/a.rs"),
                    start_line: 1,
                    end_line: 10,
                    node_count: 20,
                    hash: 123,
                    kind: "function_item".to_string(),
                },
                Fragment {
                    file: PathBuf::from("src/b.rs"),
                    start_line: 5,
                    end_line: 15,
                    node_count: 20,
                    hash: 123,
                    kind: "function_item".to_string(),
                },
            ],
            node_count: 20,
            similarity: 1.0,
        };

        // Just ensure it doesn't panic.
        print_report(&[group], &[]);
    }
}
