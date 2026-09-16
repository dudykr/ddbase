# goexec

An experimental, cooperative Rust executor with delayed handoff of explicitly
marked blocking calls. Tasks remain stackless `Send` futures. A blocking call
runs on the current OS thread; a monitor can lend its execution permit to
another worker while the call is outstanding. No Tokio patches, syscall hooks,
stack switching, or unsafe code are used in this crate.

`goexec` 0.1.0 is an unpublished workspace crate. Linux, macOS and Windows are
supported with the repository's pinned nightly toolchain.

## Use

```rust
use std::time::Duration;
use goexec::{fs, Runtime};

fn main() -> std::io::Result<()> {
    let runtime = Runtime::builder()
        .parallelism(4)
        .max_threads(512)
        .handoff_delay(Duration::from_micros(100))
        .build()?;

    let result = runtime.block_on(async {
        let first = goexec::spawn(async { fs::read_to_string("Cargo.toml") });
        let second = goexec::spawn(async { fs::metadata("Cargo.lock") });
        let contents = first.await.unwrap()?;
        let metadata = second.await.unwrap()?;
        Ok::<_, std::io::Error>((contents.len(), metadata.len()))
    })?;
    println!("{result:?}");
    assert!(runtime.shutdown_timeout(Duration::from_secs(5)));
    Ok(())
}
```

Run the self-contained file example from the workspace:

```sh
cargo run -p goexec --example files
```

`Runtime::spawn`, cloned `Handle::spawn`, and `goexec::spawn` inside a task
produce awaitable `JoinHandle<T>` values with `Result<T, JoinError>` outputs.
Handles can be awaited by other executors. Dropping a join handle detaches its
task. `Runtime::block_on` submits its root future to a worker, so both the root
future and its output must be `Send + 'static`; nested `block_on` on any goexec
worker panics. Spawned futures and their outputs have the same bounds.

`blocking(|| operation())` is synchronous. It can borrow local buffers and
non-`Send` values, without moving the closure to another thread or requiring
`'static`. Only the outermost nested boundary is tracked. Outside goexec it
simply invokes the closure. The calling future and its stack stay on the same
thread until that poll returns, even when its permit is handed off.

`fs::{read, read_to_string, write, metadata}` wrap synchronous standard-library
operations. `fs::File` supports `open`, `create`, `metadata`, `sync_all`, `Read`,
`Write`, `Seek`, and conversion to/from `std::fs::File`. Its close operation is
also tracked. Whole-file helpers track the entire operation, potentially
covering several syscalls. Operations after `into_std()` are no longer tracked.

## Scheduling and limits

Defaults are available CPU count for `parallelism`,
`max(512, parallelism + 1)` for `max_threads`, and 100 microseconds for
`handoff_delay`. Parallelism and delay must be positive, and `max_threads` must
exceed parallelism. The cap counts all workers, including blocked workers and
spares, but excludes the single monitor thread.

The scheduler uses a shared FIFO queue and a mutex/condition variable. It starts
`parallelism + 1` workers. Extra workers are created as needed, then reused until
shutdown. At most `parallelism` workers hold permits for user execution outside
marked blocking regions. Code inside a marked region must actually be blocking;
putting CPU work there can oversubscribe the machine.

The common path is TLS bookkeeping and atomic syscall-generation transitions:
it does not allocate a task, enqueue work, wake another thread, or read a clock
per call. A monitor samples at the configured delay. After observing the same
generation for at least that delay, it can reclaim the permit if work is waiting
and a replacement thread is available. A first observation starts the interval;
100 microseconds is not an upper bound on handoff latency. OS timer resolution
and scheduling affect it. The monitor parks when the runtime is idle.

Return and reclamation race via an atomic generation-tagged state. A reclaimed
caller waits in a FIFO return queue to reacquire a permit before continuing user
code. Returning callers take priority over fresh queue work. The original future
is never polled, moved or destroyed concurrently with its outstanding poll.

At the thread cap, or if additional thread creation fails, the existing call
keeps its permit and queued work may stall until a call returns. This is a hard
cap, not a deadlock avoidance guarantee: a blocked call depending on queued work
can deadlock at capacity. `Runtime::metrics()` / `Handle::metrics()` expose
threads, permits, queue/task counts, handoffs, capacity delays and spawn failures.
The latter two count attempted handoffs, not unique syscalls.

CPU loops must yield cooperatively (`goexec::yield_now().await`). There is no
forced preemption. `join!`, `select!` and timeout branches within the **same
task** cannot advance while one branch blocks; spawn independent tasks when
they need independent progress.

This crate does not replace the entire Tokio API. Version 0.1 has no network or
timer driver, Tokio integration layer, work stealing, or `!Send` task support.
Using a Tokio API which needs its runtime context on these workers is unsupported.

## Cancellation and shutdown

`JoinHandle::abort()` requests cancellation at a poll boundary. An in-progress
syscall, CPU loop, or destructor is not forcibly interrupted. Completion can
win a race with cancellation. A task's poll/destructor panic becomes a panic
`JoinError`; a root panic is resumed on the `block_on` caller. As with other Rust
runtimes, `panic = "abort"` and a double panic during unwinding cannot be isolated.

Dropping `Runtime` closes admission, requests cancellation for all tasks and
returns without joining workers. Pending tasks are woken to run cancellation.
`shutdown_timeout(self, duration)` also waits for cleanup, including worker
thread-local destructors, and reports whether
workers and the monitor finished within the timeout; it must be called outside
goexec workers. When it returns `false`, surviving threads retain the scheduler,
tasks and borrowed buffers until their current polls return. A retained `Handle`
can observe counters but cannot reopen the runtime: later spawns return cancelled
join handles and never poll the submitted futures.

## Validation

```sh
cargo fmt --all -- --check
cargo clippy -p goexec --all-targets --locked -- -D warnings
cargo test -p goexec --locked
cargo test -p goexec --locked --features loom --lib
```

Unit tests inject monitor observations and advance an explicit `Instant` instead
of depending on short sleeps. Channels/condition variables establish syscall
entry, return ordering and cleanup; generous timeouts detect failures. An
integration test uses an actual blocking TCP read with one execution permit.
Loom tests exercise the production atomic state machine and bounded models of
its locked permit and park/wake protocols, rather than the whole executor or
`async-task` internals. CI runs ordinary tests on all three supported OSes and
the Loom models on Linux. Keep the lockfile when using the pinned old nightly;
its generator version is selected for Windows compiler compatibility.

## Comparison benchmark

```sh
cargo bench -p goexec --locked --bench compare -- \
    --parallelism 4 --lanes 16 --iterations 2000 > goexec.csv
```

The standalone harness compares direct synchronous operations on a persistent
thread pool, goexec, Tokio `spawn_blocking`, and Tokio `block_in_place`.
Workloads are empty boundaries, cached 4 KiB reads, real socket-read waits,
CPU work, and a mix (one wait, one read, two CPU operations). Socket peers delay
responses by 500 microseconds; their threads are load generators excluded from
executor worker counts and timings. This uses the standard library for sockets;
it does not add networking support to goexec.

All executors have the same CPU parallelism and lane count. The direct pool has
exactly that many workers. Goexec and Tokio have the same total OS-thread cap;
Tokio's separate blocking-thread cap excludes its regular workers. CPU work runs
on executor workers without a blocking annotation/offload in every async mode.
Async lanes yield between operations. These choices make CPU rows executor
comparisons and blocking rows comparisons of the respective blocking APIs.

CSV records initialization separately (`init`), then warms each engine with up
to 32 operations per lane before measuring (`steady`). Fixture/socket creation
and peer teardown are excluded. Initial creation includes runtime/pool build
latency; lazy worker creation is exercised by warmup. Steady rows report total
throughput, operation p50/p99 in nanoseconds, peak worker count since runtime
creation, and goexec handoffs during measurement (blank for other engines).
Per-operation latency starts inside the lane and includes blocking-pool
dispatch/return where applicable; initial lane queueing and cooperative yields
are included in throughput, not in latency samples. Mixed percentiles aggregate
all operation types. Timing and sample collection overhead matter for empty
boundaries; use larger runs and interpret these rows accordingly.

Results depend on OS scheduling, caching, load and hardware. No speedup is
promised and CI has no timing threshold. Short-call handoff avoidance and
independent progress during long blocking calls are correctness requirements.
