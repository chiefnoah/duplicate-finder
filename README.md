# df -- Duplicate Finder

A command-line tool that detects code clones across codebases using AST-based structural analysis. It finds exact clones (Type 1/2) and near-duplicates (Type 3) by parsing source files with [tree-sitter](https://tree-sitter.github.io/) and comparing normalised AST subtrees.

## How it works

1. **Parse** -- source files are parsed into ASTs using tree-sitter grammars.
2. **Fragment** -- meaningful AST subtrees (functions, classes, control flow blocks, etc.) are extracted and structurally hashed. Identifiers and literals are normalised, so renamed variables or changed constants don't affect the hash (Type 2 detection).
3. **Exact matching** -- fragments with identical hashes are grouped as exact clones.
4. **Near-duplicate matching** -- remaining fragments are compared pairwise using an LCS-based similarity metric over their node-kind sequences (Type 3 detection).
5. **Subsumption** -- smaller clones contained within larger ones are removed to keep results actionable.

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

## Running tests

```sh
cargo test
```
