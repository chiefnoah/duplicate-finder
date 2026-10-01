# df -- Duplicate Finder

A command-line tool that detects code clones across codebases using AST-based structural analysis. It finds exact clones (Type 1/2) and near-duplicates (Type 3) by parsing source files with [tree-sitter](https://tree-sitter.github.io/) and comparing normalised AST subtrees.

## How it works

1. **Parse** -- source files are parsed into ASTs using tree-sitter grammars.
2. **Fragment** -- meaningful AST subtrees (functions, classes, control flow blocks, etc.) are extracted and structurally hashed. Identifiers and literals are normalised, so renamed variables or changed constants don't affect the hash (Type 2 detection).
3. **Exact matching** -- fragments with identical hashes are grouped as exact clones.
4. **Near-duplicate matching** -- remaining fragments are compared pairwise using an LCS-based similarity metric over their node-kind sequences (Type 3 detection).
5. **Subsumption** -- smaller clones contained within larger ones are removed to keep results actionable.

Fragment extraction computes each node's normalized hash and subtree count once.
It retains metadata only for meaningful roots and preserves preorder.
Near-duplicate detection rejects pairs whose sequence-length ratio cannot meet the threshold.
At threshold `1.0`, it compares sequences directly.
For other thresholds, LCS excludes shared prefixes and suffixes but retains their contribution to the score.

## Supported languages

| Language   | Extensions             |
|------------|------------------------|
| Rust       | `.rs`                  |
| Python     | `.py`                  |
| JavaScript | `.js`, `.mjs`, `.cjs`, `.jsx` |
| TypeScript | `.ts`, `.tsx`          |
| Go         | `.go`                  |
| Java       | `.java`                |
| C          | `.c`, `.h`             |
| C++        | `.cpp`, `.cc`, `.cxx`, `.hpp`, `.hxx`, `.hh` |
| Kotlin     | `.kt`, `.kts`          |
| Scala      | `.scala`, `.sc`        |

## Building

Requires Rust 1.70+.

```sh
cargo build --release
```

The binary is written to `target/release/df`.

### Nix development shell

The shell provides Rust, Cargo, rustfmt, Clippy, and a C compiler for the tree-sitter grammars.
On macOS, it also provides `libiconv` and selects the Nix linker.

```sh
nix develop path:.
cargo test
cargo build --release
```

## Usage

```sh
df <path> [options]
```

### Options

| Flag | Default | Description |
|------|---------|-------------|
| `--threshold <0.0-1.0>` | `0.8` | Similarity threshold for near-duplicate detection. Lower values find more distant clones. |
| `--min-nodes <n>` | `5` | Minimum AST node count for a subtree to be considered. Raise to ignore small fragments. |
| `--extensions <ext,...>` | all supported | Comma-separated list of file extensions to scan. |

### Examples

Scan a project for clones with default settings:

```sh
df ./my-project
```

Scan only Scala and Java files:

```sh
df ./my-project --extensions scala,java
```

Lower the similarity threshold to find more distant near-duplicates:

```sh
df ./my-project --threshold 0.6
```

Ignore small code fragments:

```sh
df ./my-project --min-nodes 15
```

### Output

Results are printed to stdout, grouped by clone cluster:

```
Found 3 clone group(s)

Clone Group 1 (2 clone(s), 42 nodes, 100% similarity, exact)
  src/auth.rs:10-35
  src/admin.rs:22-47

Clone Group 2 (2 clone(s), 28 nodes, 85% similarity, near)
  src/handlers/users.rs:5-20
  src/handlers/orders.rs:8-23
```

Progress information is printed to stderr.

### Threads

The tool uses available CPU threads by default.
It processes files, near-duplicate comparisons, and exact-group subsumption checks in parallel.
Near-duplicate groups retain the same input order and similarity rules across thread counts.

To limit the worker count, set `RAYON_NUM_THREADS`:

```sh
RAYON_NUM_THREADS=4 df ./my-project
```

## Running tests

```sh
cargo test
```

## Benchmarks

The Criterion suite measures analysis stages, thread scaling, and three isolated kernels.
It does not change production code.

Run the suite:

```sh
nix develop path:.
cargo bench --bench analysis
```

Run benchmarks on an idle system.
For comparisons, keep the compiler, codebase, and worker count unchanged.

The full suite takes several minutes.
Criterion writes HTML reports to `target/criterion/report/index.html`.

| Group | Measured work |
|-------|---------------|
| `parse` | File reads and tree-sitter parser construction and execution |
| `fragments` | AST traversal, node counts, and structural hashes |
| `exact` | Fragment copies, exact groups, and subsumption checks |
| `near` | Hash counts, buckets, node lookup, sequences, LCS comparisons, and group deduplication |
| `pipeline` | All analysis stages, without directory traversal or report output |
| `nested_fragments` | Fragment extraction at depths of 8, 32, and 128 |
| `sequences` | Node-kind sequences for functions with 32, 128, and 512 statements |
| `lcs` | Similarity comparisons for sequences with 32, 128, and 512 elements |
| `lcs_identical` | Similarity comparisons for identical sequences |
| `lcs_local_edit` | Similarity comparisons for sequences with one changed element |

Stage benchmarks use 32 or 128 Rust files with one, four, or eight workers.
Worker counts are explicit and do not depend on `RAYON_NUM_THREADS`.
The `exact` fixture contains normalized duplicates.
The `near` fixture contains unique structural hashes with a threshold of `0.8`.
The `sparse` fixture uses the same files with a threshold of `1.0`, which forces unsuccessful pair comparisons.
Fixture assertions confirm these detection paths before measurement.

Fixture creation and thread-pool construction occur outside measurements.
Stage results include output destruction and thread-pool dispatch.
File reads use the operating system cache after warmup.
Compare stages within the same fixture and worker count to locate slow stages.
Kernel results explain scaling, but their input sizes differ from the stage fixtures.

Run only the sparse workload:

```sh
cargo bench --bench analysis -- sparse
```

Run a real codebase with eight workers:

```sh
DF_BENCH_PATH=/path/to/codebase cargo bench --bench analysis -- '/real/t8'
```

The real-codebase suite uses all supported extensions, `--min-nodes 5`, and a threshold of `0.8`.
It fails on unreadable files instead of silently omitting them.

For a smoke test without statistical measurements, run:

```sh
cargo bench --bench analysis -- --test
```
