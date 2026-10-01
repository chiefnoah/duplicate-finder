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
const COPY_FRAGMENTS: usize = 10_000;
const ROW_CANDIDATES: [usize; 4] = [2, 8, 32, 128];
const SYNTAX_DEFAULT: &str = "\x1b[39m";
// Match the production capture names to compare cached-query output exactly.
const SYNTAX_THEME: [(&str, &str); 12] = [
    ("attribute", "\x1b[96m"),
    ("comment", "\x1b[90m"),
    ("constant", "\x1b[96m"),
    ("function", "\x1b[94m"),
    ("keyword", "\x1b[95m"),
    ("number", "\x1b[96m"),
    ("operator", SYNTAX_DEFAULT),
    ("property", "\x1b[97m"),
    ("punctuation", SYNTAX_DEFAULT),
    ("string", "\x1b[93m"),
    ("type", "\x1b[92m"),
    ("variable", "\x1b[97m"),
];

#[derive(Clone, Copy)]
enum Dispatch {
    Serial,
    Parallel,
}

#[derive(Clone, Copy)]
enum Capacity {
    Growing,
    Reserved,
}

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

    // Cold output includes query compilation and syntax parsing for each file.
    let mut group = c.benchmark_group("display_color_cold");
    for count in DISPLAY_MEMBERS {
        group.bench_function(BenchmarkId::from_parameter(count), |b| {
            b.iter(|| {
                let mut renderer = comparison::Renderer::new(&trees, comparison::Palette::Color);
                for member in &members[1..count] {
                    black_box(renderer.pair(&members[0], member).unwrap());
                }
            });
        });
    }
    group.finish();
}

fn bench_assessment(c: &mut Criterion) {
    let corpus = Corpus::synthetic(Workload::Near, FILE_COUNTS[1]);
    let mut copies = c.benchmark_group("fragment_copy");
    copies.throughput(Throughput::Elements(corpus.fragments.len() as u64));
    copies.bench_function("128_files", |b| {
        b.iter(|| black_box(&corpus.fragments).clone())
    });
    let many = vec![corpus.fragments[0].clone(); COPY_FRAGMENTS];
    copies.throughput(Throughput::Elements(many.len() as u64));
    copies.bench_function("10000_fragments", |b| b.iter(|| black_box(&many).clone()));
    copies.finish();

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&parser::language_for_extension("rs").unwrap())
        .unwrap();
    let mut capacity = c.benchmark_group("sequence_capacity");
    for statements in SEQUENCE_LENGTHS {
        let source = format!("fn f() {{ {} }}", "let x = a + b; ".repeat(statements));
        let tree = parser.parse(&source, None).unwrap();
        let expected = similarity::extract_kind_sequence(tree.root_node(), source.as_bytes());
        for (name, allocation) in [
            ("growing", Capacity::Growing),
            ("reserved", Capacity::Reserved),
        ] {
            assert_eq!(
                reserve_sequence(tree.root_node(), expected.len(), allocation),
                expected
            );
            capacity.bench_function(BenchmarkId::new(name, statements), |b| {
                b.iter(|| {
                    reserve_sequence(black_box(tree.root_node()), expected.len(), allocation)
                });
            });
        }
    }
    capacity.finish();

    let sequences: Vec<_> = (0..=ROW_CANDIDATES[3])
        .map(|index| {
            let source = source_code(index);
            let tree = parser.parse(&source, None).unwrap();
            similarity::extract_kind_sequence(tree.root_node(), source.as_bytes())
        })
        .collect();
    let mut rows = c.benchmark_group("candidate_row");
    for threads in THREAD_COUNTS {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap();
        for count in ROW_CANDIDATES {
            let candidates = &sequences[..=count];
            let expected = select_row(candidates, Dispatch::Serial);
            assert_eq!(
                pool.install(|| select_row(candidates, Dispatch::Parallel)),
                expected
            );
            for (name, dispatch) in [
                ("serial", Dispatch::Serial),
                ("parallel", Dispatch::Parallel),
            ] {
                rows.bench_function(BenchmarkId::new(format!("{name}/t{threads}"), count), |b| {
                    b.iter(|| pool.install(|| select_row(black_box(candidates), dispatch)));
                });
            }
        }
    }
    rows.finish();

    for extension in ["rs", "js"] {
        let language = parser::language_for_extension(extension).unwrap();
        let query = match extension {
            "rs" => tree_sitter_rust::HIGHLIGHTS_QUERY.to_owned(),
            "js" => format!(
                "{}\n{}",
                tree_sitter_javascript::HIGHLIGHT_QUERY,
                tree_sitter_javascript::JSX_HIGHLIGHT_QUERY
            ),
            _ => unreachable!(),
        };
        let source = match extension {
            "rs" => source_code(0),
            "js" => "function f(a, b) { return `${a + b}`; }\n".into(),
            _ => unreachable!(),
        };
        c.bench_function(&format!("syntax_config/{extension}"), |b| {
            b.iter(|| {
                tree_sitter_highlight::HighlightConfiguration::new(
                    language.clone(),
                    extension,
                    black_box(&query),
                    "",
                    "",
                )
                .unwrap()
            });
        });
        let mut config =
            tree_sitter_highlight::HighlightConfiguration::new(language, extension, &query, "", "")
                .unwrap();
        config.configure(&SYNTAX_THEME.map(|(name, _)| name));
        let mut highlighter = tree_sitter_highlight::Highlighter::new();
        let expected: Vec<_> = syntax::spans(&source, extension)
            .unwrap()
            .into_iter()
            .map(|span| (span.bytes, span.color))
            .collect();
        assert_eq!(cached_syntax(&mut highlighter, &config, &source), expected);
        c.bench_function(&format!("syntax_cold/{extension}"), |b| {
            b.iter(|| syntax::spans(black_box(&source), extension).unwrap());
        });
        c.bench_function(&format!("syntax_reuse/{extension}"), |b| {
            b.iter(|| cached_syntax(&mut highlighter, &config, black_box(&source)));
        });
        let mut cache = syntax::Cache::default();
        let actual: Vec<_> = cache
            .spans(&source, extension)
            .unwrap()
            .into_iter()
            .map(|span| (span.bytes, span.color))
            .collect();
        assert_eq!(actual, expected);
        c.bench_function(&format!("syntax_cached/{extension}"), |b| {
            b.iter(|| cache.spans(black_box(&source), extension).unwrap());
        });
    }
}

// Benchmark-only copies isolate reservation without a production API change.
fn reserve_sequence(
    node: tree_sitter::Node,
    count: usize,
    capacity: Capacity,
) -> Vec<&'static str> {
    let mut sequence = match capacity {
        Capacity::Growing => Vec::new(),
        Capacity::Reserved => Vec::with_capacity(count),
    };
    reserve_walk(node, &mut sequence);
    sequence
}

fn reserve_walk(node: tree_sitter::Node, sequence: &mut Vec<&'static str>) {
    let kind = match node.kind() {
        "identifier"
        | "field_identifier"
        | "type_identifier"
        | "shorthand_field_identifier"
        | "property_identifier" => "ID",
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
        | "rune_literal" => "LIT",
        other => other,
    };
    sequence.push(kind);
    let mut cursor = node.walk();
    if cursor.goto_first_child() {
        loop {
            reserve_walk(cursor.node(), sequence);
            if !cursor.goto_next_sibling() {
                break;
            }
        }
    }
}

// These rows model same-kind, different-hash candidates with valid cached sequences.
fn select_row(sequences: &[Vec<&str>], dispatch: Dispatch) -> Vec<(usize, f64)> {
    let select = |index: usize| {
        let score = similarity::tree_similarity(&sequences[0], &sequences[index]);
        (score >= DEFAULT_THRESHOLD).then_some((index, score))
    };
    match dispatch {
        Dispatch::Serial => (1..sequences.len()).filter_map(select).collect(),
        Dispatch::Parallel => (1..sequences.len())
            .into_par_iter()
            .filter_map(select)
            .collect(),
    }
}

fn cached_syntax(
    highlighter: &mut tree_sitter_highlight::Highlighter,
    config: &tree_sitter_highlight::HighlightConfiguration,
    source: &str,
) -> Vec<(std::ops::Range<usize>, &'static str)> {
    let mut stack = Vec::new();
    let mut spans = Vec::new();
    for event in highlighter
        .highlight(config, source.as_bytes(), None, |_| None)
        .unwrap()
    {
        match event.unwrap() {
            tree_sitter_highlight::HighlightEvent::HighlightStart(highlight) => {
                stack.push(highlight.0)
            }
            tree_sitter_highlight::HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            tree_sitter_highlight::HighlightEvent::Source { start, end } => {
                let color = stack
                    .last()
                    .map(|&index| SYNTAX_THEME[index].1)
                    .unwrap_or(SYNTAX_DEFAULT);
                spans.push((start..end, color));
            }
        }
    }
    spans
}

criterion_group! {
    name = benchmarks;
    config = Criterion::default()
        .sample_size(SAMPLE_SIZE)
        .warm_up_time(WARMUP)
        .measurement_time(MEASUREMENT);
    targets = bench_stages, bench_kernels, bench_focused, bench_assessment
}
criterion_main!(benchmarks);
