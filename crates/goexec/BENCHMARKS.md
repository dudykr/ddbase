# Go scheduler comparison — 2026-09-17

The optimization improves the 16-permit cooperative microbenchmarks by about
2.7× in this run. It is **not a universal speedup**: CPU and I/O changes are
small or variable, and 4-permit microbenchmarks can regress. Go is faster in
single-permit blocking/mixed work and in the 16-permit mixed workload.

## Setup and interpretation

- Apple M3 Max, 16 logical/physical CPUs; macOS 26.6.2, arm64.
- Go 1.25.1; rustc 1.81.0-nightly (2024-07-20), repository-pinned toolchain.
- Baseline production scheduler: `ea3bcdb0142e51fcb5188a93be57a1fafb3822fc`.
- Release/bench builds, locked Rust dependencies, default Go GC, no CPU affinity.
- P = goexec execution permits / Go `GOMAXPROCS`; lanes = 4 × P.
- Five serial repetitions, engine order shuffled with seed 0. No profiler or
  test process ran concurrently with these measurements.
- Per-operation clocks and sample allocation disabled. Setup and up to 32 warmup
  operations per lane excluded; submission, yields and completion included.
- Nominal total operations: 1,000,000 for Yield/Empty, 100,000 for CachedRead/Cpu,
  4,096 for SyscallWait, 8,192 for Mixed; rounded up to whole lane iterations.

`Yield` is an explicitly self-waking Rust future versus Go `runtime.Gosched()`.
`Empty` additionally enters/exits a goexec blocking boundary. Go has no equivalent
empty annotation. These expose cooperative scheduling costs, not preemption or
general application speed. In Go 1.25.1, `Gosched` publishes to the global run
queue; goexec reschedules locally. See the
[Go scheduler implementation](https://github.com/golang/go/blob/go1.25.1/src/runtime/proc.go).

`SyscallWait` uses blocking descriptors and Go `syscall.Read`/`Write`, **not the
Go network poller**. External Rust peer threads delay each reply by 500 µs.
Go startup/worker/handoff metrics are omitted because they are not directly
comparable. Go uses its normal runtime thread limit; goexec has a 512-worker cap.
Cached reads compare whole-file standard-library APIs, so compiler, allocator,
GC and filesystem costs also contribute. CPU work is the same 10,000-step
multiply/rotate recurrence. Mixed repeats wait/read/CPU/CPU.

## Main results

Median operations/second (higher is better). Ratios are ratios of medians;
values below 1 indicate a slowdown. Five samples do not establish statistical
significance.

| P | Workload | Before | After | Go | After / before | After / Go |
|---:|---|---:|---:|---:|---:|---:|
| 1 | Yield | 23,921,728 | 24,164,974 | 22,015,521 | 1.01× | 1.10× |
| 1 | Empty | 21,096,564 | 20,864,431 | 22,147,331 | 0.99× | 0.94× |
| 1 | CachedRead | 99,665 | 100,566 | 91,905 | 1.01× | 1.09× |
| 1 | Cpu | 93,582 | 93,977 | 93,224 | 1.00× | 1.01× |
| 1 | SyscallWait | 2,875 | 2,873 | 4,577 | 1.00× | 0.63× |
| 1 | Mixed | 11,870 | 12,459 | 17,391 | 1.05× | 0.72× |
| 4 | Yield | 64,195,326 | 65,862,594 | 8,516,165 | 1.03× | 7.73× |
| 4 | Empty | 63,661,322 | 55,823,303 | 8,650,023 | 0.88× | 6.45× |
| 4 | CachedRead | 200,720 | 178,778 | 183,988 | 0.89× | 0.97× |
| 4 | Cpu | 358,140 | 353,883 | 353,285 | 0.99× | 1.00× |
| 4 | SyscallWait | 8,219 | 8,173 | 4,538 | 0.99× | 1.80× |
| 4 | Mixed | 32,971 | 33,263 | 19,653 | 1.01× | 1.69× |
| 16 | Yield | 22,670,896 | 61,889,166 | 3,996,276 | 2.73× | 15.49× |
| 16 | Empty | 22,864,525 | 61,790,693 | 4,026,947 | 2.70× | 15.34× |
| 16 | CachedRead | 101,053 | 101,282 | 95,819 | 1.00× | 1.06× |
| 16 | Cpu | 1,151,048 | 1,153,801 | 1,099,282 | 1.00× | 1.05× |
| 16 | SyscallWait | 22,931 | 24,053 | 17,983 | 1.05× | 1.34× |
| 16 | Mixed | 89,363 | 89,136 | 97,884 | 1.00× | 0.91× |

The roughly 15× advantage over Go in 16-permit Yield/Empty is specific to
explicit cooperative rescheduling. It does not mean goexec is 15× faster at
CPU work, normal Go networking, goroutine creation or arbitrary applications.

Go syscall results are particularly variable: at P=4 the observed range was
4,303–22,726 ops/s, and at P=16 it was 17,501–72,216 ops/s. Its fastest runs
exceeded goexec. Do not interpret the median blocking advantage as a guaranteed
latency/throughput benefit. At P=1, Go consistently won the blocking workload.

## Longer scheduler checks and tradeoff

Because million-operation microbenchmarks can finish in tens of milliseconds,
a separate randomized comparison used **32 million operations, seven repeats**.
The same production scheduler changes were tested; this preceded the final
Go-only peer-accept cleanup, which is unused by these goexec-only runs.

| P | Workload | Before ops/s | After ops/s | Change |
|---:|---|---:|---:|---:|
| 4 | Yield | 66,191,981 | 58,146,243 | -12.2% |
| 4 | Empty | 63,527,614 | 52,147,071 | -17.9% |
| 16 | Yield | 23,902,851 | 69,264,369 | +189.8% |
| 16 | Empty | 23,234,561 | 70,341,761 | +202.7% |

The 4-permit regressions are real observations, not discarded outliers. A
separate 800-million-operation P=4 Empty run with two seconds of sampling per
process was nearly equal (62.23M before, 61.77M after), showing sensitivity to
run duration and host scheduling; that profiled run is not part of the tables.
The changes favor scaling to many workers, and should not be treated as an
across-the-board improvement for small worker counts. The P=4 cached-read
main result also fell about 11%, with overlapping run ranges.

## Changes and validation

1. Keep each worker's rotated stealing-peer list until the worker set grows.
   Previously, every 64-poll checkpoint allocated a vector and cloned/dropped
   every peer's reference count while holding the scheduler lock. Workers are
   only added during runtime operation, making the cache valid between growth.
2. Publish the per-worker poll counter with an atomic store from its sole writer,
   avoiding a read-modify-write instruction on every poll. Metrics still update
   after every completed poll, including across checkpoints.

The 64-poll checkpoint, 16-poll injector check, return priority, handoff delay
and thread/permit limits are unchanged. A deterministic regression test forces
an original worker to steal a child from a worker created after its first run.
Trial cache-line padding did not consistently help and was removed.

Validation: 33 unit/integration/doc tests, six Loom models, workspace format
check, `cargo clippy -p goexec --all-targets --locked -- -D warnings`, and Go vet.
A separate latency smoke run validated all 30 workload/engine combinations,
including the Go companion built with `-race`; no races were reported.

## Reproduce and inspect

```sh
# Save the unoptimized scheduler's harness after adding the comparison CLI.
cargo bench -p goexec --locked --bench compare --no-run
# Copy the executable Cargo reports to an absolute path, then apply changes.
python3 crates/goexec/benches/compare_go.py --baseline /absolute/path/to/compare-before
# Increase duration for scheduler-only measurements:
python3 crates/goexec/benches/compare_go.py --baseline /absolute/path/to/compare-before \
    --cases Yield,Empty --scale 32 --repeats 7 --output target/goexec-long
```

The second command also measures Go; the longer goexec-only experiment above
included a cache-only intermediate build. The runner writes raw runs, median
and min/max summaries, and environment metadata to
`target/goexec-comparison/results` by default, or to the specified `--output`
directory. Generated result files are not checked into the repository.

This host had a configured but unavailable `sccache`; Rust commands were run
with `RUSTC_WRAPPER=`. No compiler flags or global configuration were changed.

See [harness options and methodology](README.md#go-scheduler-comparison).
