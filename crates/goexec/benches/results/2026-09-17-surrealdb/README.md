# SurrealDB embedded executor comparison — partial, stopped by user

The user requested stopping after **262 of 750 planned SDK runs**.
All completed runs have zero response errors. 131 of 150 conditions
have results; only 3 conditions reached all five repetitions.
The final in-flight process was terminated and is excluded from CSV summaries.
The full matched-core and original-core performance suites had not started.
The earlier original-core smoke run is not used as performance evidence.

The implementation and all 30 correctness configurations are complete. This
report preserves the available measurements without presenting them as the
completed benchmark plan. Repeat counts by condition: {1: 47, 2: 45, 3: 34, 4: 2, 5: 3}.

## Interim SDK comparison

Only a variant and Tokio run with the same workload, backend, P, concurrency,
seed, and repeat ID form a comparison pair. Medians use the common completed
repeats for each condition. Thus an unmatched final run cannot skew a ratio.
The table equally weights the observed conditions using geometric means;
missing conditions and one-sample cells limit its representativeness.
Throughput above 1 is faster; p99 below 1 is lower latency.

| Backend | Variant | Paired conditions | Run pairs | Throughput / Tokio | Condition ratio range | p99 / Tokio |
| --- | --- | --- | --- | --- | --- | --- |
| memory | goexec / existing policy | 26 | 54 | 0.926× | 0.705–1.053× | 1.159× |
| rocksdb | goexec / existing policy | 26 | 51 | 0.914× | 0.250–1.191× | 1.135× |
| rocksdb | goexec / handoff | 26 | 51 | 1.101× | 0.251–3.275× | 0.934× |

The existing-policy goexec configuration is slower in this partial aggregate.
RocksDB handoff improves the aggregate, but individual conditions include large
regressions. These observations do not establish a general speedup, and the
small/incomplete repeat counts cannot support statistical significance claims.

| Backend | Variant | P | Paired conditions | Throughput / Tokio |
| --- | --- | --- | --- | --- |
| memory | goexec / existing policy | 1 | 8 | 0.924× |
| memory | goexec / existing policy | 4 | 9 | 0.972× |
| memory | goexec / existing policy | 16 | 9 | 0.883× |
| rocksdb | goexec / existing policy | 1 | 10 | 1.006× |
| rocksdb | goexec / existing policy | 4 | 10 | 1.020× |
| rocksdb | goexec / existing policy | 16 | 6 | 0.648× |
| rocksdb | goexec / handoff | 1 | 10 | 1.654× |
| rocksdb | goexec / handoff | 4 | 10 | 1.023× |
| rocksdb | goexec / handoff | 16 | 6 | 0.631× |

| Backend | Variant | Extreme | Workload | P | Lanes | Paired repeats | Throughput / Tokio |
| --- | --- | --- | --- | --- | --- | --- | --- |
| memory | goexec / existing policy | lowest | read | 16 | 16 | 2 | 0.705× |
| memory | goexec / existing policy | highest | range | 16 | 16 | 1 | 1.053× |
| rocksdb | goexec / existing policy | lowest | update | 16 | 16 | 1 | 0.250× |
| rocksdb | goexec / existing policy | highest | create | 4 | 16 | 1 | 1.191× |
| rocksdb | goexec / handoff | lowest | update | 16 | 16 | 1 | 0.251× |
| rocksdb | goexec / handoff | highest | read | 1 | 1 | 2 | 3.275× |

Use [paired summary CSV](sdk-paired-summary.csv) for comparisons. It includes
common repeat IDs, min/median/max throughput, and p50/p99. The [condition table](sdk-details.md)
and [raw summary](sdk-summary.csv) retain every completed condition and its
repeat count, RSS, threads, and handoffs. Their direct ratio column uses each
group's available samples; the paired summary above takes precedence where
repeat counts differ. [Raw CSV](sdk-runs.csv) retains every completed run.

## Reproduction, validation, and limits

- SurrealDB v3.2.4: `93ab219d69f09d8f999851b0359c80ebe6726102`.
- Goexec [PR #107](https://github.com/dudykr/ddbase/pull/107), exact measured
  runtime dependency: `e57e96903c7c33a1c38fb34ee3c2ff11ed579145`. Later PR commits package tests,
  reproduction, and results without changing that measured runtime.
- Host: `machdep.cpu.brand_string: Apple M5 Max; hw.ncpu: 18; hw.memsize: 137438953472`; `macOS-26.6.2-arm64-arm-64bit`.
  Rust 1.95.0; release opt-level 3, LTO off, 16 codegen units, panic abort,
  system allocator. [Environment and binary hashes](sdk-environment.json).
- Every completed process seeded 100,000 records with 960 deterministic
  alphanumeric payload bytes per record, warmed up for 3 seconds, and measured
  for 10 seconds. Seed 20260917; serial shuffled execution; P=1,4,16 and lanes
  P or 4P. No compiler/test/other benchmark ran alongside measurements.
- Point read, create, version update, 80/20 read/update mix, and indexed
  32-record range. Writes use disjoint lane-owned keys. Setup, seed, and
  shutdown are excluded from timing. Closed-loop request latency includes SDK
  queueing and response extraction; SQL/payload construction is excluded from
  latency but contributes to throughput. There is no coordinated-omission correction.
- Storage uses sync every, P RocksDB background threads, max(4,P) affinity
  workers, P Rayon threads, and an inline reserve of two. Actual datastore
  worker capacity is asserted. Both main executors have a 512-thread cap,
  with native pools and goexec monitor/timer threads additional.
- The existing-policy variant retains RocksDB inline/offload dispatch.
  Handoff replaces only InlineGuard dispatch with goexec::blocking; explicit
  count/compaction affinity jobs remain. Warm-cache local results do not
  characterize cold disk or network servers. The host was not exclusively
  isolated or CPU-pinned.
- Peak RSS covers the whole process including setup/cleanup. Faster creates
  insert more records, so their RSS does not isolate executor overhead at an
  equal row count. Threads are sampled every 20 ms. Handoffs cover only the
  timed phase.
- No Tokio runtime is constructed in goexec runs; runtime-independent Tokio
  primitives remain. Native networking, remote stores, scripting, ML, WASM,
  extension runtimes, bucket/file APIs, and SDK import/export are unvalidated.
- [Correctness validation](verification.json): all 30 SDK/core, memory/RocksDB,
  runtime, and worker configurations passed CRUD, rollback, forced-index lookup,
  SLEEP/query timeout, external cancellation, background shutdown, and RocksDB
  reopen persistence. A separate 150-condition short smoke matrix also passed.
- Goexec passed pinned-nightly and Rust 1.95 tests, fmt, clippy, and six Loom
  models. CI passed Linux/macOS/Windows. Timer/budget tests cover cancellation,
  reset, concurrent waits, missed ticks, shutdown, single-worker progress, and
  the inability to preempt a synchronous poll.

The [patch](../../surrealdb/surrealdb.patch) and [rerun commands](../../surrealdb/README.md)
are retained. Patch SHA-256: `1549add9c7f178a6ad7b4e1a459f9f1b866d8c6060be5d4fb7d10ae3557a3326`. A fresh pinned checkout
accepted the patch and matched all 46 changed source files byte-for-byte.
Resume the same runner/output directory to finish missing repetitions; no
measurements resume automatically. Use `report.py --allow-partial` to recreate
this archive. [Process logs](process-logs.tar.gz) include the interrupted final
process; [stop metadata](stopped-by-user.json) records why it was excluded.

Successful requests in the completed timed phases:
204,139,379.
