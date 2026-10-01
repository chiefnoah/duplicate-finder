# Optimization pass

## Corrected baseline

The baseline uses the corrected detector from commit `34ee36e`.
The benchmark suite includes focused pair and source-output cases.

- Host: Apple M4 Pro, 14 logical CPUs, macOS, `aarch64-apple-darwin`.
- Compiler: Rust 1.98.1 from the project development shell.
- Profile: optimized Cargo benchmark profile.
- Samples: 10, with a 0.1-second warmup and a 0.2-second measurement target.
- Synthetic detection corpus: 32 or 128 Rust files, with 16 statements per function.
- Threads: 1, 4, and 8.
- Threshold: `0.8` for exact and near corpora, `1.0` for sparse corpora.
- Source output: 2 or 32 members, with 2,048 padding functions per file and warm syntax caches.

At commit `fbc4590`, run the baseline:

```sh
nix develop path:. --command cargo bench --bench analysis -- \
  --save-baseline corrected --sample-size 10 \
  --warm-up-time 0.1 --measurement-time 0.2
```

These short measurements guide this pass. They do not represent production workloads.
Criterion stores full measurements under `target/criterion`.

| Case | Baseline estimate |
|---|---:|
| Near detection, exact corpus, 128 files, 1 thread | 3.482 ms |
| Near detection, near corpus, 128 files, 1 thread | 4.443 ms |
| Near detection, sparse corpus, 128 files, 1 thread | 4.964 ms |
| Pipeline, exact corpus, 128 files, 1 thread | 11.100 ms |
| Pipeline, near corpus, 128 files, 1 thread | 12.149 ms |
| Pipeline, sparse corpus, 128 files, 1 thread | 12.060 ms |
| Pipeline, near corpus, 128 files, 8 threads | 2.869 ms |
| Sequence extraction, 512 statements | 383.920 µs |
| Shifted LCS, 512 tokens | 740.850 µs |
| Near detection, 2 files, 1 thread | 62.736 µs |
| Plain source output, 32 members | 1.695 ms |
| Color source output, 32 members | 2.233 ms |

## Individual changes

Focused comparisons after `tsk-13` use 20 samples, a 0.2-second warmup, and a 0.5-second measurement target.

| Task | Focused case | Before | After |
|---|---|---:|---:|
| `tsk-12`: skip same-hash buckets | Near detection, exact corpus, 128 files, 1 thread | 3.482 ms | 10.825 µs |
| `tsk-13`: borrow node-kind strings | Sequence extraction, 512 statements | 383.920 µs | 300.860 µs |
| `tsk-14`: reuse pair scores | Near detection, 2 files, 1 thread | 53.507 µs | 52.158 µs |
| `tsk-15`: shorter rows without clearing | Shifted LCS, 512 tokens | 716.350 µs | 695.520 µs |
| `tsk-15`: shorter rows without clearing | Asymmetric LCS, 128 then 512 tokens | 183.800 µs | 178.040 µs |
| `tsk-16`: seek syntax spans | Color source output, 32 members | 2.073 ms | 1.387 ms |
| `tsk-17`: reuse reference tokens | Plain source output, 32 members | 1.267 ms | 707.660 µs |
| `tsk-17`: reuse reference tokens | Color source output, 32 members | 1.418 ms | 778.380 µs |

The single-row LCS experiment passed equivalence tests but increased several kernel times by 4–9%.
The retained implementation uses two rows and omits redundant clearing.
Both rows use the shorter middle sequence for their width.

The pair-score measurement falls within the Criterion noise threshold.
This change removes duplicate LCS work, but this local-edit fixture shows no clear speedup.

The pale-overlay change preceded `tsk-13`.
Later color-output comparisons must use the pale-overlay version as their baseline.

The reference-token cache holds only the current group reference.
The display benchmark resets this cache for each measured group.
Its syntax caches remain warm, as in the earlier measurements.

## Combined detection results

The final run uses the same short measurement settings as the corrected baseline.
All cases use 128 files.

| Case | Threads | Corrected baseline | Final estimate |
|---|---:|---:|---:|
| Exact corpus pipeline | 1 | 11.100 ms | 7.652 ms |
| Near corpus pipeline | 1 | 12.149 ms | 10.374 ms |
| Sparse corpus pipeline | 1 | 12.060 ms | 11.109 ms |
| Near corpus pipeline | 8 | 2.869 ms | 2.778 ms |
| Near detection, near corpus | 1 | 4.443 ms | 3.469 ms |
| Near detection, sparse corpus | 1 | 4.964 ms | 4.084 ms |

The new asymmetric LCS cases use the separate `rows` baseline from before `tsk-15`.
The combined run excludes those cases from comparisons with `corrected`.

Validation: 60 tests and 116 benchmark smoke cases passed.
Clippy reports the existing sort and recursive-parameter warnings.

## Next-pass assessment

The measured benchmark harness and detector inputs now correspond to commit `de6ac0c`.
Its detector modules have formatting-only changes relative to `4630f91`.
Concurrent CLI and Nix changes remain outside this assessment.
The new benchmark cases do not change production detection or output.

The `round2` baseline uses 20 samples, a 0.2-second warmup, and a 0.5-second measurement target.
The host and compiler remain the same as in the first pass.

```sh
nix develop path:. --command cargo bench --bench analysis -- \
  --save-baseline round2 --sample-size 20 \
  --warm-up-time 0.2 --measurement-time 0.5
```

The assessment isolates these costs:

- Fragment copies for 128 files and a synthetic list of 10,000 fragments.
- Cold color output, including query compilation and source parsing.
- Syntax configuration creation versus benchmark-only configuration reuse.
- Serial versus parallel selection for 2, 8, 32, and 128 candidates.
- Sequence extraction with growing versus preallocated vectors.

The syntax experiment checks source ranges and foreground colors against production output.
The capacity experiment checks complete node-kind sequences against the production extractor.
The candidate experiment checks identical accepted indices and scores for both dispatch modes.
It models valid, different-hash candidates at threshold `0.8`, not the complete greedy detector.

### Current measurements

| Case | Estimate |
|---|---:|
| Exact corpus pipeline, 128 files, 1 thread | 8.020 ms |
| Near corpus pipeline, 128 files, 1 thread | 10.815 ms |
| Sparse corpus pipeline, 128 files, 1 thread | 11.365 ms |
| Near corpus pipeline, 128 files, 8 threads | 2.720 ms |
| Exact detection, exact corpus, 128 files, 1 thread | 792.620 µs |
| Copy 256 fragments from 128 files | 6.570 µs |
| Copy 10,000 synthetic fragments | 297.158 µs |
| Cold color output, 32 files | 565.290 ms |
| Rust syntax configuration creation | 8.921 ms |
| Rust cold highlighting, 16 statements | 9.011 ms |
| Rust highlighting with reused configuration and highlighter | 51.405 µs |
| JavaScript cold highlighting, small function | 3.766 ms |
| JavaScript highlighting with reused configuration and highlighter | 8.537 µs |

The reuse measurements describe benchmark-only experiments, not implemented production speedups.
Large-file syntax parsing remains necessary after query caching.
Fragment-copy savings cannot exceed their measured cost on this corpus.
The copy measurement includes destruction of the copied fragments.

### Decisions

| Task | Decision | Reason and effort |
|---|---|---|
| `tsk-20`: syntax configurations | Proceed first | Configuration compilation dominates cold output. A language cache needs a small syntax-service change and output equivalence tests. |
| `tsk-21`: serial candidate rows | Investigate selectively | Small rows benefit on some worker counts. A safe dispatch policy needs long-sequence cases and whole-pipeline evidence. |
| `tsk-19`: borrowed fragments | Defer | Copies consume about 0.06% of the near pipeline in this corpus. The API and containment changes exceed the measured CPU benefit. |
| `tsk-22`: reserved sequence vectors | Defer | The extraction experiment improved by 3–5% at smaller sizes but regressed by about 3% at 512 statements. |
| `tsk-23`: containment index | Defer pending a profile | Exact detection consumes about 10% of the single-thread exact pipeline. Its containment rules require more implementation and test work than a simple cache. |

On eight workers, a row with eight candidates took 16.316 µs serially versus 26.407 µs in parallel.
A row with 128 candidates took 383.918 µs serially versus 99.178 µs in parallel.
These measurements include the same thread-pool entry cost for both modes.
They do not establish a cutoff for different sequence lengths or match distributions.

These decisions apply to the measured synthetic corpora.
A large real-code corpus or memory profile can change the deferred priorities.
All 156 benchmark smoke cases passed, including the experiment-equivalence assertions.
The benchmark input hashes remained unchanged throughout the baseline run.
Validation also passed all 62 current tests. Clippy reports only the existing warnings.

## `tsk-20`: production syntax cache

Each report now keeps one compiled configuration per query grammar and reuses its highlighter.
Extension aliases share a configuration. TypeScript and TSX remain separate.
The cache initializes lazily and stays local to the renderer.
Plain output does not create a highlighter or compile queries.
Kotlin retains its existing AST fallback without query compilation.
The cache inserts configurations only after successful compilation.

| Case | `round2` baseline | After cache |
|---|---:|---:|
| Cold color output, 2 files | 34.890 ms | 26.058 ms |
| Cold color output, 32 files | 565.290 ms | 273.200 ms |
| Small Rust file, cold versus cached API | 9.011 ms | 51.570 µs |
| Small JavaScript file, cold versus cached API | 3.766 ms | 8.505 µs |

The complete cold-output benchmark includes the first configuration compilation, source parsing, alignment, and text output construction.
The cached API measurements exclude first-file compilation.
Warm file-span-cache output shows no significant change.
All measurements use the same settings as `round2`.

Equivalence tests cover supported languages, extension aliases, Unicode, malformed source, empty source, and recovery after unsupported extensions.
Existing plain and ANSI snapshots also pass.
All 66 tests and 158 benchmark smoke cases passed. Clippy reports only the existing warnings.
