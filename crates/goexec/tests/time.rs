#![cfg(feature = "time")]

use std::{
    future::pending,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    time::{Duration, Instant},
};

use goexec::{time, Runtime};

const LIMIT: Duration = Duration::from_secs(10);

#[test]
fn timers_progress_concurrently_without_a_tokio_runtime() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (sent, received) = mpsc::channel();
    let _task = rt.spawn(async move {
        let tasks: Vec<_> = (0..128)
            .map(|_| {
                goexec::spawn(async {
                    let deadline = Instant::now() + Duration::from_millis(2);
                    time::sleep_until(deadline).await;
                    assert!(Instant::now() >= deadline);
                    1
                })
            })
            .collect();
        let mut sum = 0;
        for task in tasks {
            sum += task.await.unwrap();
        }
        sent.send(sum).unwrap();
    });
    assert_eq!(received.recv_timeout(LIMIT).unwrap(), 128);
    assert!(rt.shutdown_timeout(LIMIT));
}

#[test]
fn timeout_cancels_pending_work_and_ready_value_wins() {
    struct Dropped(Arc<AtomicBool>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    rt.block_on(async {
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let result = time::timeout(Duration::ZERO, async move {
            let _guard = guard;
            pending::<()>().await;
        })
        .await;
        assert_eq!(result, Err(time::Elapsed));
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(time::timeout_at(Instant::now(), async { 42 }).await, Ok(42));
        assert_eq!(time::timeout(Duration::MAX, async { 7 }).await, Ok(7));
    });
    assert!(rt.shutdown_timeout(LIMIT));
}

#[test]
fn reset_and_intervals_preserve_deadlines_and_cancel_safely() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    rt.block_on(async {
        let mut sleep = time::sleep(Duration::from_secs(3600));
        assert!(futures::poll!(&mut sleep).is_pending());
        sleep.reset(Instant::now());
        sleep.await;
        let mut ticks = time::interval(Duration::from_millis(10));
        ticks.set_missed_tick_behavior(time::MissedTickBehavior::Delay);
        let first = ticks.tick().await;
        assert!(time::timeout(Duration::ZERO, ticks.tick()).await.is_err());
        let second = ticks.tick().await;
        assert_eq!(second.duration_since(first), Duration::from_millis(10));
    });
    assert!(rt.shutdown_timeout(LIMIT));
}

#[test]
fn shutdown_drops_long_lived_timers() {
    struct Dropped(mpsc::Sender<()>);
    impl Drop for Dropped {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (sent, received) = mpsc::channel();
    let guard = Dropped(sent);
    let task = rt.spawn(async move {
        let _guard = guard;
        time::sleep(Duration::from_secs(3600)).await;
    });
    assert!(rt.shutdown_timeout(LIMIT));
    received.recv_timeout(LIMIT).unwrap();
    assert!(futures::executor::block_on(task)
        .unwrap_err()
        .is_cancelled());
}

#[test]
fn timer_wakes_a_sibling_while_another_worker_is_blocked() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (release, wait) = mpsc::channel();
    let (entered, entry) = mpsc::channel();
    let (done, completion) = mpsc::channel();
    let _task = rt.spawn(async move {
        entered.send(()).unwrap();
        goexec::blocking(|| wait.recv_timeout(LIMIT).unwrap());
        done.send(()).unwrap();
    });
    entry.recv_timeout(LIMIT).unwrap();
    let _task = rt.spawn(async move {
        time::sleep(Duration::from_millis(2)).await;
        release.send(()).unwrap();
    });
    completion.recv_timeout(LIMIT).unwrap();
    assert!(rt.metrics().handoffs > 0);
    assert!(rt.shutdown_timeout(LIMIT));
}

#[test]
fn timeout_does_not_interrupt_a_blocking_poll() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    rt.block_on(async {
        let result = time::timeout(Duration::ZERO, async {
            goexec::blocking(|| std::thread::sleep(Duration::from_millis(5)));
            42
        })
        .await;
        assert_eq!(result, Ok(42));
    });
    assert!(rt.shutdown_timeout(LIMIT));
}

#[test]
fn cooperative_cpu_loop_allows_a_sibling_to_cancel_it() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (sent, received) = mpsc::channel();
    let iterations = Arc::new(AtomicUsize::new(0));
    let progress = iterations.clone();
    let loop_task = rt.spawn(async move {
        loop {
            progress.fetch_add(1, Ordering::Relaxed);
            goexec::consume_budget().await;
        }
    });
    let _task = rt.spawn(async move {
        loop_task.abort();
        assert!(loop_task.await.unwrap_err().is_cancelled());
        sent.send(()).unwrap();
    });
    received.recv_timeout(LIMIT).unwrap();
    assert!(rt.shutdown_timeout(LIMIT));
}

#[test]
#[should_panic(expected = "interval period must be nonzero")]
fn zero_interval_is_rejected() {
    time::interval(Duration::ZERO);
}
