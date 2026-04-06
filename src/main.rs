use anyhow::Result;
use clap::Parser;
use rayon::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;
use walkdir::WalkDir;

mod clones;
mod hasher;
mod parser;
mod reporter;
mod similarity;

#[derive(Parser, Debug)]
#[command(name = "df", about = "Duplicate finder — detect code clones in codebases")]
pub struct Cli {
    /// Path to scan for duplicates
    pub path: PathBuf,

    /// Similarity threshold for near-duplicate detection (0.0–1.0)
    #[arg(long, default_value = "0.8")]
    pub threshold: f64,

    /// Minimum AST node count for a subtree to be considered
    #[arg(long, default_value = "5")]
    pub min_nodes: usize,

    /// Filter by file extensions (comma-separated, e.g. "rs,py,js")
    #[arg(long, value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,
}

pub fn collect_files(cli: &Cli) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(&cli.path).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e,
            None => continue,
        };
        if let Some(ref exts) = cli.extensions {
            if !exts.iter().any(|e| e == ext) {
                continue;
            }
        } else if parser::language_for_extension(ext).is_none() {
            continue;
        }
        files.push(path.to_path_buf());
    }
    Ok(files)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let files = collect_files(&cli)?;

    eprintln!("Scanning {} files...", files.len());

    // Phase 1: Parse all files in parallel.
    let parsed: Vec<_> = files
        .par_iter()
        .filter_map(|path| match parser::parse_file(path) {
            Ok((tree, source)) => Some((path.clone(), tree, source)),
            Err(e) => {
                eprintln!("warning: {e}");
                None
            }
        })
        .collect();

    eprintln!("Parsed {} files", parsed.len());

    // Phase 2: Collect fragments in parallel.
    let all_fragments: Vec<_> = parsed
        .par_iter()
        .flat_map(|(path, tree, source)| {
            hasher::collect_fragments(tree, source, path, cli.min_nodes)
        })
        .collect();

    eprintln!("Collected {} AST fragments", all_fragments.len());

    // Phase 3: Find exact clone groups (Type 1/2).
    let exact_groups = clones::find_clone_groups(all_fragments.clone());

    // Phase 4: Find near-duplicate groups (Type 3).
    let trees_map: HashMap<PathBuf, (tree_sitter::Tree, String)> = parsed
        .into_iter()
        .map(|(path, tree, source)| (path, (tree, source)))
        .collect();

    let near_groups =
        similarity::find_near_duplicates(&all_fragments, &trees_map, cli.threshold, cli.min_nodes);

    // Phase 5: Report.
    reporter::print_report(&exact_groups, &near_groups);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_collect_files_finds_files_in_directory() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("b.py"), "print('hello')").unwrap();
        fs::create_dir(dir.path().join("sub")).unwrap();
        fs::write(dir.path().join("sub/c.js"), "console.log('hi')").unwrap();

        let cli = Cli {
            path: dir.path().to_path_buf(),
            threshold: 0.8,
            min_nodes: 5,
            extensions: None,
        };

        let files = collect_files(&cli).unwrap();
        assert_eq!(files.len(), 3);
    }

    #[test]
    fn test_collect_files_filters_by_extension() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("a.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("b.py"), "print('hello')").unwrap();
        fs::write(dir.path().join("c.txt"), "not code").unwrap();

        let cli = Cli {
            path: dir.path().to_path_buf(),
            threshold: 0.8,
            min_nodes: 5,
            extensions: Some(vec!["rs".to_string(), "py".to_string()]),
        };

        let files = collect_files(&cli).unwrap();
        assert_eq!(files.len(), 2);
    }
}
