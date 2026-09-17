# Go scheduler comparison — 2026-09-17

Latest application comparison: [SurrealDB v3.2.4 embedded SDK](benches/results/2026-09-17-surrealdb/README.md).
That separate experiment includes 262 completed runs and 30 correctness
configurations; measurement stopped before all planned repetitions completed.

This is an archived report for the optimization in
`01ed65c34b235f6a9dec7c7baa6903e11b575f53`. The peer-list and cancellation-waker
optimizations were subsequently reverted after native Rspack builds showed
application-level regressions. Its "After" numbers describe that historical
revision, not the current implementation. The benchmarks and regression tests
remain available.

Caching cancellation-waker registration improves sustained cooperative work:
Yield is 13% faster at one permit and 35% faster at four permits, while the
16-permit result is essentially unchanged. Empty blocking boundaries improve
12–51% by median. CPU and I/O workloads show little consistent change; the
longer 16-permit mixed workload is 3.2% slower. This is not a universal speedup.

These results compare that historical change against
`79d7a27d545ee3435b18fdbbe597e0b3eff22fdd`, which already includes stealing-peer
caching and per-worker poll stores. The earlier optimization's results remain
in [the previous report](https://github.com/dudykr/ddbase/blob/79d7a27d545ee3435b18fdbbe597e0b3eff22fdd/crates/goexec/BENCHMARKS.md).

## Setup and interpretation

- Apple M3 Max, 16 physical/logical CPUs; macOS 26.6.2, arm64.
- Installed Go 1.25.1; repository-pinned rustc 1.81.0-nightly (2024-07-20).
  This is not a comparison against the latest available Go release.
- Optimized builds, locked Rust dependencies, default Go GC, no CPU affinity.
- P = goexec execution permits / Go `GOMAXPROCS`; lanes = 4 × P.
- Main comparison: five repetitions, engines run serially in shuffled order,
  seed 0. No test/compiler/profiler process ran alongside timed measurements.
- No per-operation clocks or latency sample allocations in throughput runs.
  Setup and up to 32 warmup operations per lane are excluded. Submission,
  yields, result collection and socket close are included.
- Main nominal operations: 1,000,000 Yield/Empty; 100,000 CachedRead/Cpu;
  4,096 SyscallWait; 8,192 Mixed, rounded up to whole lane iterations.

`Yield` compares a self-waking Rust future with `runtime.Gosched()`. In the
installed Go version, `Gosched` puts the goroutine on the global run queue;
goexec reschedules locally. See the
[Go 1.25.1 scheduler implementation](https://github.com/golang/go/blob/go1.25.1/src/runtime/proc.go).
`Empty` additionally enters/exits goexec's blocking annotation; Go has no
corresponding empty annotation. Neither case measures asynchronous preemption,
goroutine creation, normal Go networking or general application performance.

`SyscallWait` uses blocking descriptors and Go `syscall.Read`/`Write`, not the Go
network poller. External Rust peer threads delay replies by 500 µs. Go uses its
normal runtime thread limit; goexec caps workers at 512. Go startup, worker and
handoff metrics are omitted because matching counters are unavailable.
CachedRead compares whole-file APIs on a cached 4 KiB fixture, Cpu uses the same
10,000-step multiply/rotate recurrence, and Mixed repeats wait/read/CPU/CPU.
Compiler, standard library, allocator, GC and OS scheduling costs contribute.

## Main results

Median operations/second; higher is better. Ratios divide medians. Raw runs,
ranges and environment/binary hashes are saved under
[results/2026-09-17-cancellation](benches/results/2026-09-17-cancellation/).
Five samples do not establish statistical significance.

| P | Workload | Before | After | Go | After / before | After / Go |
|---:|---|---:|---:|---:|---:|---:|
| 1 | Yield | 24,339,043 | 25,717,794 | 22,036,422 | 1.06× | 1.17× |
| 1 | Empty | 21,591,665 | 23,523,093 | 22,291,926 | 1.09× | 1.06× |
| 1 | CachedRead | 100,130 | 99,437 | 91,177 | 0.99× | 1.09× |
| 1 | Cpu | 87,194 | 87,123 | 91,655 | 1.00× | 0.95× |
| 1 | SyscallWait | 3,417 | 3,422 | 5,502 | 1.00× | 0.62× |
| 1 | Mixed | 11,268 | 12,248 | 20,522 | 1.09× | 0.60× |
| 4 | Yield | 74,617,355 | 81,368,342 | 8,486,611 | 1.09× | 9.59× |
| 4 | Empty | 57,609,355 | 80,211,219 | 8,538,567 | 1.39× | 9.39× |
| 4 | CachedRead | 181,912 | 179,336 | 183,649 | 0.99× | 0.98× |
| 4 | Cpu | 349,584 | 349,508 | 352,487 | 1.00× | 0.99× |
| 4 | SyscallWait | 9,535 | 9,585 | 5,467 | 1.01× | 1.75× |
| 4 | Mixed | 37,587 | 39,086 | 24,432 | 1.04× | 1.60× |
| 16 | Yield | 81,875,774 | 91,574,739 | 3,932,847 | 1.12× | 23.28× |
| 16 | Empty | 80,928,522 | 82,766,041 | 3,932,322 | 1.02× | 21.05× |
| 16 | CachedRead | 101,127 | 100,495 | 96,218 | 0.99× | 1.04× |
| 16 | Cpu | 1,169,897 | 1,156,423 | 1,089,827 | 0.99× | 1.06× |
| 16 | SyscallWait | 26,783 | 26,295 | 22,081 | 0.98× | 1.19× |
| 16 | Mixed | 99,459 | 102,550 | 115,496 | 1.03× | 0.89× |

The roughly 21–23× Go comparison in 16-permit Yield/Empty applies only to these
explicit rescheduling microbenchmarks. CPU work is close. Go wins the
single-permit blocking/mixed workloads and 16-permit Mixed in this run.
Blocking results are especially variable: inspect `main-runs.csv` and the
min/max columns in `main-summary.csv` before drawing conclusions.

## Sustained scheduler and mixed checks

Short microbenchmarks finish in milliseconds. The follow-up uses **32 million
operations and seven repetitions** for each Yield/Empty cell, comparing only
the two Rust executables, serially with shuffled order. Median M ops/s and
observed min–max ranges:

| P | Workload | Before M ops/s (min–max) | After M ops/s (min–max) | Change |
|---:|---|---:|---:|---:|
| 1 | Yield | 24.71 (23.83–25.57) | 27.95 (27.08–28.25) | +13.1% |
| 1 | Empty | 21.00 (20.81–21.37) | 23.44 (23.20–23.70) | +11.6% |
| 4 | Yield | 64.51 (60.79–69.45) | 87.07 (71.77–90.10) | +35.0% |
| 4 | Empty | 58.18 (53.18–62.88) | 68.61 (50.76–82.75) | +17.9% |
| 16 | Yield | 88.77 (66.30–113.16) | 88.50 (74.26–111.43) | -0.3% |
| 16 | Empty | 62.11 (59.06–86.67) | 93.53 (85.72–119.14) | +50.6% |

P=1 Yield/Empty and P=4 Yield have non-overlapping observed ranges. Other cells
have more host-scheduling variability. In particular, the P=16 Yield gain in
the short run does not persist in this longer test. The measured optimization
removes repeated registration work, but does not guarantee improved scaling
on every workload or machine.

A separate **32,768-operation Mixed run, five repetitions** checks that the
scheduler-only results do not hide blocking regressions:

| P | Before ops/s | After ops/s | Change |
|---:|---:|---:|---:|
| 1 | 12,591 | 13,027 | +3.5% |
| 4 | 39,705 | 39,940 | +0.6% |
| 16 | 104,314 | 101,004 | -3.2% |

The P=16 Mixed slowdown is reported rather than discarded. Its before/after
ranges overlap; it is not evidence of a general I/O improvement. A separate
latency smoke run (`--scale 0.1 --repeats 1 --latency`) completed all 54
P/workload/engine combinations and recorded p50/p99 in `latency-runs.csv`.
Those short instrumented runs validate the sampling path, not tail-latency
performance claims.

## Implementation and validation

Previously, `Abortable<F>` registered the same waker through `AtomicWaker` after
every pending poll. `TaskFuture` now stores the pinned user future separately
from an `Abortable<Pending<()>>` cancellation signal and caches the registered
waker. When `Waker::will_wake` matches, only the abort flag is checked. A new
waker still polls `Abortable`, using its existing register/check race handling.
Only abort consumes this registration, and the abort flag stays set afterwards.

Cancellation is checked before polling user code and after Pending. Poll and
future/output-destructor panics remain isolated; completion may still win an
abort race. The cost is one extra cached Waker per task (and a clone on first
registration or waker change). No unsafe code, new dependency or custom atomic
cancellation protocol was added.

The 64-poll permit checkpoint, 16-poll injector check, return priority, handoff
delay and thread cap are unchanged. An earlier candidate removed the periodic
permit release: its initial gains did not persist consistently in seven long
repetitions, so it was reverted before the measurements above.

Regression coverage includes abort before the first poll, abort after 1,024
self-wakes followed by parking, shutdown of a continuously self-waking task,
external injection within 16 polls, and returning callers after repeated polls.
The existing blocking-borrow, panic, FIFO return, worker-growth and concurrent
wake tests remain in place.

Validation: 36 unit/integration/doc tests, six Loom models, workspace format
check, Clippy with warnings denied, Go vet, and the latency smoke run above.
Loom still models the existing production call-state and bounded scheduler
protocols; it does not model the entire executor or futures' AtomicWaker.

## Reproduce

Build the baseline at the revision above with the existing harness and save
the executable Cargo reports before rebuilding the changed crate:

```sh
RUSTC_WRAPPER= cargo bench -p goexec --locked --bench compare --no-run
# Copy the reported executable to /absolute/path/to/compare-before.
# Return to the changed checkout for the following commands.
RUSTC_WRAPPER= python3 crates/goexec/benches/compare_go.py \
    --baseline /absolute/path/to/compare-before \
    --output target/goexec-comparison/cancellation-main
RUSTC_WRAPPER= python3 crates/goexec/benches/compare_go.py \
    --baseline /absolute/path/to/compare-before --engines goexec,baseline \
    --cases Yield,Empty --scale 32 --repeats 7 \
    --output target/goexec-comparison/cancellation-long
RUSTC_WRAPPER= python3 crates/goexec/benches/compare_go.py \
    --baseline /absolute/path/to/compare-before --engines goexec,baseline \
    --cases Mixed --scale 4 --repeats 5 \
    --output target/goexec-comparison/cancellation-mixed
RUSTC_WRAPPER= python3 crates/goexec/benches/compare_go.py \
    --baseline /absolute/path/to/compare-before --scale 0.1 --repeats 1 --latency \
    --output target/goexec-comparison/cancellation-latency
```

`--rust-binary` and `--go-binary` reuse existing executables; the saved environment
files record the actual options and SHA-256 hashes used. Snapshots precede later
test/documentation-only edits. RUSTC_WRAPPER was cleared for these builds because
the configured sccache executable was unavailable. No global settings changed.
See [harness options and methodology](README.md#go-scheduler-comparison).
