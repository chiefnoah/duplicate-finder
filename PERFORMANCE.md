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

Run the baseline:

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

| Task | Focused case | Before | After |
|---|---|---:|---:|
| `tsk-12`: skip same-hash buckets | Near detection, exact corpus, 128 files, 1 thread | 3.482 ms | 10.825 µs |
| `tsk-13`: borrow node-kind strings | Sequence extraction, 512 statements | 383.920 µs | 300.860 µs |

The pale-overlay change preceded `tsk-13`.
Later color-output comparisons must use the pale-overlay version as their baseline.
