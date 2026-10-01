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
