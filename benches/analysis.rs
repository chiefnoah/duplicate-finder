use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::hint::black_box;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;
use tree_sitter::Tree;

// Reuse production modules. The custom harness does not run their unit tests.
#[allow(dead_code, unused_imports)]
#[path = "../src/clones.rs"]
mod clones;
#[allow(dead_code, unused_imports)]
#[path = "../src/comparison.rs"]
mod comparison;
#[allow(dead_code, unused_imports)]
#[path = "../src/hasher.rs"]
mod hasher;
#[allow(dead_code, unused_imports)]
#[path = "../src/parser.rs"]
mod parser;
#[allow(dead_code, unused_imports)]
#[path = "../src/similarity.rs"]
mod similarity;
#[allow(dead_code, unused_imports)]
#[path = "../src/syntax.rs"]
mod syntax;

const MIN_NODES: usize = 5;
const DEFAULT_THRESHOLD: f64 = 0.8;
const STRICT_THRESHOLD: f64 = 1.0;
const FILE_COUNTS: [usize; 2] = [32, 128];
const THREAD_COUNTS: [usize; 3] = [1, 4, 8];
const STATEMENTS: usize = 16;
const NESTING_DEPTHS: [usize; 3] = [8, 32, 128];
const SEQUENCE_LENGTHS: [usize; 3] = [32, 128, 512];
const REAL_PATH_ENV: &str = "DF_BENCH_PATH";
const SAMPLE_SIZE: usize = 20;
const WARMUP: Duration = Duration::from_secs(1);
const MEASUREMENT: Duration = Duration::from_secs(3);
const KIND_VARIANTS: usize = 7;
const DISPLAY_MEMBERS: [usize; 2] = [2, 32];
const PADDING_FUNCTIONS: usize = 2048;

type Parsed = Vec<(PathBuf, Tree, String)>;

#[derive(Clone, Copy)]
enum Stage {
    Parse,
    Fragments,
    Exact,
    Near,
    Pipeline,
}

impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Parse => "parse",
            Self::Fragments => "fragments",
            Self::Exact => "exact",
            Self::Near => "near",
            Self::Pipeline => "pipeline",
        }
    }

    fn run(self, corpus: &Corpus) {
        match self {
            Self::Parse => {
                black_box(parse_files(black_box(&corpus.files)));
            }
            Self::Fragments => {
                black_box(collect_fragments(black_box(&corpus.parsed)));
            }
            Self::Exact => {
                // Include the fragment copy that main performs for exact detection.
                black_box(clones::find_clone_groups(
                    black_box(&corpus.fragments).clone(),
                ));
            }
            Self::Near => {
                black_box(similarity::find_near_duplicates(
                    black_box(&corpus.fragments),
                    black_box(&corpus.trees),
                    corpus.threshold,
                    MIN_NODES,
                ));
            }
            Self::Pipeline => {
                let parsed = parse_files(black_box(&corpus.files));
                let fragments = collect_fragments(&parsed);
                let exact = clones::find_clone_groups(fragments.clone());
                let trees = parsed
                    .into_iter()
                    .map(|(path, tree, source)| (path, (tree, source)))
                    .collect();
                let near = similarity::find_near_duplicates(
                    &fragments,
                    &trees,
                    corpus.threshold,
                    MIN_NODES,
                );
                black_box((exact, near));
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Workload {
    Exact,
    Near,
    Sparse,
}

impl Workload {
    fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Near => "near",
            Self::Sparse => "sparse",
        }
    }

    fn threshold(self) -> f64 {
        match self {
            Self::Sparse => STRICT_THRESHOLD,
            _ => DEFAULT_THRESHOLD,
        }
    }
}

struct Corpus {
    _directory: Option<TempDir>,
    name: String,
    files: Vec<PathBuf>,
    parsed: Parsed,
    fragments: Vec<hasher::Fragment>,
    trees: HashMap<PathBuf, (Tree, String)>,
    threshold: f64,
}

impl Corpus {
    fn new(name: String, files: Vec<PathBuf>, threshold: f64) -> Self {
        let parsed = parse_files(&files);
        let fragments = collect_fragments(&parsed);
        let trees = parsed
            .iter()
            .map(|(path, tree, source)| (path.clone(), (tree.clone(), source.clone())))
            .collect();

        Self {
            _directory: None,
            name,
            files,
            parsed,
            fragments,
            trees,
            threshold,
        }
    }

    fn synthetic(workload: Workload, count: usize) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let files: Vec<_> = (0..count)
            .map(|index| {
                let path = directory.path().join(format!("{index}.rs"));
                let variant = match workload {
                    Workload::Exact => 0,
                    _ => index,
                };
                std::fs::write(&path, source_code(variant)).unwrap();
                path
            })
            .collect();
        let mut corpus = Self::new(
            format!("{}/{count}", workload.name()),
            files,
            workload.threshold(),
        );
        corpus._directory = Some(directory);

        // Prevent normalized exact clones from bypassing the near workloads.
        let functions: Vec<_> = corpus
            .fragments
            .iter()
            .filter(|fragment| fragment.kind == "function_item")
            .collect();
        assert_eq!(functions.len(), count);
        let hashes: HashSet<_> = functions.iter().map(|fragment| fragment.hash).collect();
        let expected = match workload {
            Workload::Exact => 1,
            _ => count,
        };
        assert_eq!(hashes.len(), expected);
        assert!(corpus
            .parsed
            .iter()
            .all(|(_, tree, _)| !tree.root_node().has_error()));

        // Check that each fixture exercises its intended detection path.
        let exact = clones::find_clone_groups(corpus.fragments.clone());
        let near = similarity::find_near_duplicates(
            &corpus.fragments,
            &corpus.trees,
            corpus.threshold,
            MIN_NODES,
        );
        match workload {
            Workload::Exact => {
                assert!(!exact.is_empty());
                assert!(near.is_empty());
            }
            Workload::Near => {
                assert!(exact.is_empty());
                assert!(!near.is_empty());
            }
            Workload::Sparse => {
                assert!(exact.is_empty());
                assert!(near.is_empty());
            }
        }

        corpus
    }
}

fn source_code(variant: usize) -> String {
    let mut source = String::from("fn f(a: i32, b: i32) {\n");
    for index in 0..STATEMENTS {
        let operator = if (variant >> index) & 1 == 0 {
            "-"
        } else {
            "+"
        };
        source.push_str(&format!("    let x{index} = a {operator} b;\n"));
    }
    source.push_str("}\n");
    source
}

fn parse_files(files: &[PathBuf]) -> Parsed {
    files
        .par_iter()
        .map(|path| {
            let (tree, source) = parser::parse_file(path).unwrap();
            (path.clone(), tree, source)
        })
        .collect()
}

fn collect_fragments(parsed: &Parsed) -> Vec<hasher::Fragment> {
    parsed
        .par_iter()
        .flat_map(|(path, tree, source)| hasher::collect_fragments(tree, source, path, MIN_NODES))
        .collect()
}

fn real_corpus(path: &Path) -> Corpus {
    assert!(
        path.exists(),
        "DF_BENCH_PATH does not exist: {}",
        path.display()
    );
    let files: Vec<_> = walkdir::WalkDir::new(path)
        .into_iter()
        .map(|entry| entry.unwrap())
        .filter(|entry| entry.path().is_file())
        .map(|entry| entry.into_path())
        .filter(|path| {
            path.extension()
                .and_then(|extension| extension.to_str())
                .and_then(parser::language_for_extension)
                .is_some()
        })
        .collect();
    assert!(
        !files.is_empty(),
        "DF_BENCH_PATH contains no supported files"
    );
    Corpus::new("real".into(), files, DEFAULT_THRESHOLD)
}

fn bench_stages(c: &mut Criterion) {
    // Fixture creation, precomputed inputs, and thread-pool startup stay outside timing.
    let pools: Vec<_> = THREAD_COUNTS
        .into_iter()
        .map(|count| {
            rayon::ThreadPoolBuilder::new()
                .num_threads(count)
                .build()
                .unwrap()
        })
        .collect();
    let mut corpora = Vec::new();
    for workload in [Workload::Exact, Workload::Near, Workload::Sparse] {
        for count in FILE_COUNTS {
            corpora.push(Corpus::synthetic(workload, count));
        }
    }
    if let Some(path) = std::env::var_os(REAL_PATH_ENV) {
        corpora.push(real_corpus(Path::new(&path)));
    }

    for stage in [
        Stage::Parse,
        Stage::Fragments,
        Stage::Exact,
        Stage::Near,
        Stage::Pipeline,
    ] {
        let mut group = c.benchmark_group(stage.name());
        for corpus in &corpora {
            group.throughput(Throughput::Elements(corpus.files.len() as u64));
            for (threads, pool) in THREAD_COUNTS.into_iter().zip(&pools) {
                let id = BenchmarkId::new(&corpus.name, format!("t{threads}"));
                group.bench_function(id, |b| {
                    b.iter(|| pool.install(|| stage.run(corpus)));
                });
            }
        }
        group.finish();
    }
}

fn bench_kernels(c: &mut Criterion) {
    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&parser::language_for_extension("rs").unwrap())
        .unwrap();
    let mut fragments = c.benchmark_group("nested_fragments");
    for depth in NESTING_DEPTHS {
        let source = format!(
            "fn f() {{ {} let x = 1; {} }}",
            "{ ".repeat(depth),
            "} ".repeat(depth)
        );
        let tree = parser.parse(&source, None).unwrap();
        assert!(!tree.root_node().has_error());
        fragments.bench_function(BenchmarkId::from_parameter(depth), |b| {
            b.iter(|| {
                hasher::collect_fragments(
                    black_box(&tree),
                    black_box(&source),
                    Path::new("nested.rs"),
                    MIN_NODES,
                )
            });
        });
    }
    fragments.finish();

    let mut sequences = c.benchmark_group("sequences");
    for statements in SEQUENCE_LENGTHS {
        let source = format!("fn f() {{ {} }}", "let x = a + b; ".repeat(statements));
        let tree = parser.parse(&source, None).unwrap();
        sequences.bench_function(BenchmarkId::from_parameter(statements), |b| {
            b.iter(|| {
                similarity::extract_kind_sequence(
                    black_box(tree.root_node()),
                    black_box(source.as_bytes()),
                )
            });
        });
    }
    sequences.finish();

    let mut lcs = c.benchmark_group("lcs");
    for length in SEQUENCE_LENGTHS {
        let a: Vec<_> = (0..length)
            .map(|index| format!("kind_{}", index % KIND_VARIANTS))
            .collect();
        let b: Vec<_> = (0..length)
            .map(|index| format!("kind_{}", (index + 1) % KIND_VARIANTS))
            .collect();
        lcs.bench_function(BenchmarkId::from_parameter(length), |bench| {
            bench.iter(|| similarity::tree_similarity(black_box(&a), black_box(&b)));
        });
    }
    lcs.finish();

    let mut asymmetric = c.benchmark_group("lcs_asymmetric");
    for length in SEQUENCE_LENGTHS {
        let long: Vec<_> = (0..length)
            .map(|index| format!("kind_{}", index % KIND_VARIANTS))
            .collect();
        let short: Vec<_> = (0..length / 4)
            .map(|index| format!("kind_{}", (index + 1) % KIND_VARIANTS))
            .collect();
        for (name, a, b) in [
            ("short_first", &short, &long),
            ("long_first", &long, &short),
        ] {
            asymmetric.bench_function(BenchmarkId::new(name, length), |bench| {
                bench.iter(|| similarity::tree_similarity(black_box(a), black_box(b)));
            });
        }
    }
    asymmetric.finish();

    // Distinguish shared-end fast paths from the shifted-sequence worst case.
    for (name, replacement) in [
        ("lcs_identical", None),
        ("lcs_local_edit", Some("different_kind")),
    ] {
        let mut group = c.benchmark_group(name);
        for length in SEQUENCE_LENGTHS {
            let a: Vec<_> = (0..length)
                .map(|index| format!("kind_{}", index % KIND_VARIANTS))
                .collect();
            let mut b = a.clone();
            if let Some(kind) = replacement {
                b[length / 2] = kind.into();
            }
            group.bench_function(BenchmarkId::from_parameter(length), |bench| {
                bench.iter(|| similarity::tree_similarity(black_box(&a), black_box(&b)));
            });
        }
        group.finish();
    }
}

fn bench_focused(c: &mut Criterion) {
    let pair = Corpus::synthetic(Workload::Near, 2);
    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .build()
        .unwrap();
    c.bench_function("near_pair/t1", |b| {
        b.iter(|| pool.install(|| Stage::Near.run(&pair)));
    });

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&parser::language_for_extension("rs").unwrap())
        .unwrap();
    let padding = "fn padding() { let x = 1; }\n".repeat(PADDING_FUNCTIONS);
    let mut trees = HashMap::new();
    let mut members = Vec::new();
    for index in 0..DISPLAY_MEMBERS[1] {
        let source = format!(
            "{padding}fn member{index}() {{ {} }}\n",
            "let α = a + b; ".repeat(STATEMENTS)
        );
        let tree = parser.parse(&source, None).unwrap();
        let file = PathBuf::from(format!("display{index}.rs"));
        let fragment = hasher::collect_fragments(&tree, &source, &file, MIN_NODES)
            .into_iter()
            .rfind(|fragment| fragment.kind == "function_item")
            .unwrap();
        members.push(fragment);
        trees.insert(file, (tree, source));
    }
    for (name, palette) in [
        ("display_plain", comparison::Palette::Plain),
        ("display_color", comparison::Palette::Color),
    ] {
        let mut group = c.benchmark_group(name);
        for count in DISPLAY_MEMBERS {
            let mut renderer = comparison::Renderer::new(&trees, palette);
            // Warm syntax caches to isolate repeated comparison and span traversal costs.
            for member in &members[1..count] {
                black_box(renderer.pair(&members[0], member).unwrap());
            }
            group.bench_function(BenchmarkId::from_parameter(count), |b| {
                b.iter(|| {
                    renderer.begin_group();
                    for member in &members[1..count] {
                        black_box(
                            renderer
                                .pair(black_box(&members[0]), black_box(member))
                                .unwrap(),
                        );
                    }
                });
            });
        }
        group.finish();
    }
}

criterion_group! {
    name = benchmarks;
    config = Criterion::default()
        .sample_size(SAMPLE_SIZE)
        .warm_up_time(WARMUP)
        .measurement_time(MEASUREMENT);
    targets = bench_stages, bench_kernels, bench_focused
}
criterion_main!(benchmarks);
