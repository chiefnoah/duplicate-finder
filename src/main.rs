use anyhow::{Context, Result};
use clap::{Parser, ValueEnum};
use rayon::prelude::*;
use std::collections::HashMap;
use std::io::IsTerminal;
use std::path::PathBuf;
use walkdir::WalkDir;

mod clones;
mod comparison;
mod hasher;
mod parser;
mod reporter;
mod similarity;
mod syntax;

#[derive(Clone, Copy, Debug, ValueEnum)]
enum ColorChoice {
    Auto,
    Always,
    Never,
}

fn palette(choice: ColorChoice) -> comparison::Palette {
    match choice {
        ColorChoice::Always => comparison::Palette::Color,
        ColorChoice::Never => comparison::Palette::Plain,
        ColorChoice::Auto => {
            let disabled = std::env::var_os("NO_COLOR").is_some_and(|value| !value.is_empty());
            if std::io::stdout().is_terminal() && !disabled {
                comparison::Palette::Color
            } else {
                comparison::Palette::Plain
            }
        }
    }
}

const THRESHOLD_RANGE: std::ops::RangeInclusive<f64> = 0.0..=1.0;

fn parse_threshold(value: &str) -> std::result::Result<f64, String> {
    let threshold = value.parse::<f64>().map_err(|error| error.to_string())?;
    if !threshold.is_finite() || !THRESHOLD_RANGE.contains(&threshold) {
        return Err("Threshold must be finite and between 0.0 and 1.0.".into());
    }
    Ok(threshold)
}

#[derive(Parser, Debug)]
#[command(
    name = "df",
    about = "Duplicate finder — detect code clones in codebases"
)]
pub struct Cli {
    /// Path to scan for duplicates
    pub path: PathBuf,

    /// Similarity threshold for near-duplicate detection (0.0–1.0)
    #[arg(long, default_value = "0.8", value_parser = parse_threshold)]
    pub threshold: f64,

    /// Minimum AST node count for a subtree to be considered
    #[arg(long, default_value = "5")]
    pub min_nodes: usize,

    /// Filter by file extensions (comma-separated, e.g. "rs,py,js")
    #[arg(long, value_delimiter = ',')]
    pub extensions: Option<Vec<String>>,

    /// Show structurally matched source tokens with syntax colors
    #[arg(long)]
    show_similarities: bool,

    /// Exit with an error when clone groups are found
    #[arg(long)]
    fail_on_clones: bool,

    /// Color policy for source comparisons
    #[arg(long, value_enum, default_value = "auto")]
    color: ColorChoice,
}

pub fn collect_files(cli: &Cli) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for entry in WalkDir::new(&cli.path) {
        let entry = entry.with_context(|| format!("Cannot scan {}", cli.path.display()))?;
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
    if cli.show_similarities {
        reporter::print_comparisons(&exact_groups, &near_groups, &trees_map, palette(cli.color))?;
    }

    if cli.fail_on_clones && (!exact_groups.is_empty() || !near_groups.is_empty()) {
        anyhow::bail!("Clone groups found.");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn reject_missing_scan_path() {
        let directory = tempfile::tempdir().unwrap();
        let cli = Cli {
            path: directory.path().join("missing"),
            threshold: 0.8,
            min_nodes: 5,
            extensions: None,
            show_similarities: false,
            fail_on_clones: false,
            color: ColorChoice::Auto,
        };
        assert!(collect_files(&cli).is_err());
    }

    #[test]
    fn reject_invalid_thresholds() {
        for value in ["NaN", "inf", "-inf", "-0.1", "1.1"] {
            let option = format!("--threshold={value}");
            assert!(
                Cli::try_parse_from(["df", ".", &option]).is_err(),
                "{value}"
            );
        }
    }

    #[test]
    fn accept_threshold_endpoints() {
        for value in ["0", "1", "0.8"] {
            let option = format!("--threshold={value}");
            assert!(Cli::try_parse_from(["df", ".", &option]).is_ok());
        }
    }

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
            show_similarities: false,
            fail_on_clones: false,
            color: ColorChoice::Auto,
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
            show_similarities: false,
            fail_on_clones: false,
            color: ColorChoice::Auto,
        };

        let files = collect_files(&cli).unwrap();
        assert_eq!(files.len(), 2);
    }
}
