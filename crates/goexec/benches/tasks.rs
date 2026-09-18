//! Fixed-size task batches; build each revision before paired measurements.
use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use futures::future::join_all;
use goexec::{blocking, spawn, yield_now, Runtime};

async fn batch(case: &str) -> u64 {
    let (tasks, yields) = match case {
        "spawn" => (20_000, 0),
        "yield" | "empty" => (1_000, 100),
        "blocking" => (128, 0),
        _ => panic!("expected spawn, yield, empty, or blocking"),
    };
    let empty = case == "empty";
    let wait = case == "blocking";
    let handles = (0..tasks)
        .map(|_| {
            spawn(async move {
                for _ in 0..yields {
                    if empty {
                        blocking(|| black_box(()));
                    }
                    yield_now().await;
                }
                if wait {
                    blocking(|| std::thread::sleep(Duration::from_millis(1)));
                }
                black_box(1_u64)
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        join_all(handles)
            .await
            .into_iter()
            .map(Result::unwrap)
            .sum::<u64>(),
        tasks
    );
    tasks * (yields + 1)
}

fn main() {
    let mut args = std::env::args().skip(1);
    let case = args.next().unwrap_or_else(|| "spawn".into());
    let parallelism = args.next().map(|s| s.parse().unwrap()).unwrap_or(4);
    let duration = args.next().map(|s| s.parse().unwrap()).unwrap_or(500);
    let runtime = Runtime::builder()
        .parallelism(parallelism)
        .max_threads(usize::MAX)
        .build()
        .unwrap();
    let handle = runtime.handle();
    let entered = handle.enter();
    handle.block_on(batch(&case));
    let before = runtime.metrics();
    let start = Instant::now();
    let mut iterations = 0;
    let mut operations = 0;
    loop {
        operations += handle.block_on(batch(&case));
        iterations += 1;
        if iterations >= 3 && start.elapsed() >= Duration::from_millis(duration) {
            break;
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    let after = runtime.metrics();
    println!("case,parallelism,seconds,iterations,throughput,workers,handoffs");
    println!(
        "{case},{parallelism},{},{iterations},{},{},{}",
        elapsed / iterations as f64,
        operations as f64 / elapsed,
        after.spawned_threads,
        after.handoffs - before.handoffs
    );
    drop(entered);
    runtime.shutdown();
}
