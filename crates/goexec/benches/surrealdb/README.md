# SurrealDB embedded executor comparison

The experiment pins SurrealDB v3.2.4 at
`93ab219d69f09d8f999851b0359c80ebe6726102` and goexec PR
[#107](https://github.com/dudykr/ddbase/pull/107) at
`e57e96903c7c33a1c38fb34ee3c2ff11ed579145`. The upstream patch includes the
runtime adapter, correctness harness, benchmark binary, and adapted original
core Criterion read/create routines. It does not modify the upstream repository
or publish a SurrealDB PR.

## Variants

- `tokio`: default SurrealDB task/timer execution and RocksDB inline/offload policy.
- `goexec`: goexec tasks, cooperative checkpoints, and timers; the existing
  RocksDB inline/offload policy remains active.
- `goexec-blocking`: the same goexec runtime, with RocksDB's
  `InlineGuard::try_inline_or_offload` storage operations executed inside
  `goexec::blocking`. This replaces that guard's reserve-based dispatch with
  delayed handoff. Other explicit affinity-pool operations, including full
  counts and compaction, retain their existing pools.

Tokio channels, locks, cancellation tokens, select/join macros, and async I/O
traits remain dependencies. No Tokio runtime is constructed in goexec runs.
CPU sorting stays on Rayon; isolated owned file writes use an independent
goexec task with a blocking boundary. The optional timer driver is async-io's
process-global thread. Native networking, cloud/file bucket APIs, file-backed
full-text mappers, SDK file import/export, scripting, ML, WASM, and extension
runtimes are outside the validated experiment. This patch is not a drop-in
replacement for every SurrealDB feature combination.

The SDK harness uses the existing `unstable_from_datastore` entry point. Requests
still traverse the SDK's native router and its spawned query tasks. Constructing
the datastore explicitly lets both SDK and core paths pass the actual executor
width to `with_runtime_worker_threads`. The harness verifies RocksDB's resulting
inline capacity. Connection/setup time is not part of throughput or latency.

## Reproduce

Requirements: git, Rust 1.95.0, a working native RocksDB build environment,
Python 3.9+, and `psutil==7.0.0`. Run from the ddbase workspace:

```sh
python3 -m venv target/surrealdb-integration/venv
target/surrealdb-integration/venv/bin/pip install psutil==7.0.0

PYTHON=target/surrealdb-integration/venv/bin/python
RUNNER=crates/goexec/benches/surrealdb/run.py

$PYTHON $RUNNER prepare
$PYTHON $RUNNER build
$PYTHON $RUNNER verify --access sdk,core \
  --output target/surrealdb-integration/verification

# Short execution/validation smoke test; not a performance result.
$PYTHON $RUNNER bench --records 1000 --warmup 0.1 --seconds 0.3 --repeats 1 \
  --output target/surrealdb-integration/smoke

# Full SDK matrix: 750 serial processes, 150 groups, five repeats per group.
$PYTHON $RUNNER bench --output target/surrealdb-integration/results

# Original core Criterion workloads: 75 serial processes, reads and creates.
$PYTHON $RUNNER classic --output target/surrealdb-integration/classic
```

All builds use optimized release code with `opt-level=3`, `lto=false`,
`codegen-units=16`, `panic=abort`, and the system allocator. The original
Criterion routines are built as the `native-sdb` binary in the same profile,
avoiding a separate test-profile comparison. Their original small random values
and read predicate differ from the SDK matrix; compare variants within each
suite, not their absolute throughput against each other.

The default paths can be overridden with `--source`, `--bins`, and `--output`.
Successful rows can be resumed by rerunning the same command. The runner rejects
changed binary fingerprints in an existing results directory. Invalid attempts
remain in the raw CSV and are excluded from summaries. It stops on the first
invalid run so an integration failure cannot become a performance conclusion.

## Measurement contract

The SDK suite starts each process with 100,000 records. Each contains a numeric
key, an indexed numeric field, a version counter, and 960 deterministic
alphanumeric payload bytes. A fixed xorshift seed determines payloads and
request keys. Payloads differ by record. Workloads are point reads, creates,
updates, 80/20 read/update mixes, and indexed 32-record range reads. Each lane
owns disjoint update keys; creates use distinct IDs. Successful responses and
initial/create row counts are checked.

Each group uses 3 seconds of warmup and 10 seconds of timed execution, repeated
five times in a seeded shuffled order. Worker widths are 1, 4, and 16, with P
and 4P concurrent closed-loop lanes. Runtime/pool construction, database setup,
seeding, warmup, post-run validation, and shutdown are excluded from steady
timing. The denominator includes draining the last in-flight requests.

Latency starts immediately before query submission and ends after checked
response extraction, including SDK queueing. It excludes SQL string/payload
generation and the lane's initial release delay. Every measured request is
recorded in a three-significant-digit HDR histogram. These are closed-loop
service latencies; they are not open-loop overload or coordinated-omission-
corrected measurements.

Both variants use `sync=every`, P RocksDB background threads, `max(4,P)` affinity
workers (the upstream minimum is four), P Rayon threads, and a 512-thread
executor cap. Goexec's monitor and timer driver and the native storage/CPU pools
are additional process threads. The first two variants retain the upstream
inline reserve of two workers; the third deliberately replaces that policy.

`peak_rss_bytes` is the OS-reported process-lifetime peak, including setup and
cleanup. `peak_threads_sampled` includes all process threads and is sampled at
20 ms, so very short-lived peaks can be missed. Create workloads grow the
dataset, including during warmup; faster runs insert more records, so their RSS
is not an equal-record-count estimate of executor overhead. Handoffs and
RocksDB grant/divert counts are deltas around only the timed phase.

Results describe a warm-cache local experiment. OS cache flushing and hardware
isolation are not performed. Raw CSV, per-process stdout/stderr, binary hashes,
compiler/hardware metadata, and median/min/max summaries are retained.
