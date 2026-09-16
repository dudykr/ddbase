use std::sync::{atomic::Ordering, mpsc};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(5);

fn runtime(max_threads: usize) -> Runtime {
    let mut builder = Runtime::builder().parallelism(1).max_threads(max_threads);
    builder.manual_monitor = true;
    builder.build().unwrap()
}

fn blocked(rt: &Runtime) -> (JoinHandle<()>, mpsc::Sender<()>) {
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let task = rt.spawn(async move {
        blocking(|| {
            entered.send(()).unwrap();
            released.recv().unwrap();
        });
    });
    started.recv_timeout(TIMEOUT).unwrap();
    (task, release)
}

fn scan(rt: &Runtime) {
    let mut observations = Vec::new();
    let mut state = rt.shared.state.lock().unwrap();
    let now = Instant::now();
    rt.shared.scan(&mut state, &mut observations, now);
    rt.shared
        .scan(&mut state, &mut observations, now + rt.shared.handoff_delay);
}

fn wait_state(rt: &Runtime, predicate: impl Fn(&Scheduler) -> bool) {
    let state = rt.shared.state.lock().unwrap();
    let (state, _) = rt
        .shared
        .changed
        .wait_timeout_while(state, TIMEOUT, |s| !predicate(s))
        .unwrap();
    assert!(predicate(&state), "scheduler condition timed out");
}

#[test]
fn quick_and_nested_calls_do_not_handoff() {
    let rt = runtime(2);
    assert_eq!(
        rt.block_on(async {
            for _ in 0..10_000 {
                assert_eq!(blocking(|| blocking(|| 42)), 42);
            }
            7
        }),
        7
    );
    let metrics = rt.metrics();
    assert_eq!(metrics.handoffs, 0);
    assert_eq!(metrics.spawned_threads, 2);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn handoff_needs_elapsed_observation_and_queued_work() {
    let rt = runtime(2);
    let (task, release) = blocked(&rt);
    scan(&rt);
    assert_eq!(rt.metrics().handoffs, 0);

    let (done, completion) = mpsc::channel();
    let other = rt.spawn(async move {
        done.send(()).unwrap();
    });
    let now = Instant::now();
    let mut observations = Vec::new();
    {
        let mut state = rt.shared.state.lock().unwrap();
        rt.shared.scan(&mut state, &mut observations, now);
        rt.shared.scan(&mut state, &mut observations, now);
    }
    assert_eq!(rt.metrics().handoffs, 0);
    assert!(completion.try_recv().is_err());
    {
        let mut state = rt.shared.state.lock().unwrap();
        rt.shared
            .scan(&mut state, &mut observations, now + rt.shared.handoff_delay);
    }
    completion.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(rt.metrics().handoffs, 1);
    release.send(()).unwrap();
    futures::executor::block_on(task).unwrap();
    futures::executor::block_on(other).unwrap();
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn capacity_waits_without_losing_the_original_permit() {
    let rt = runtime(2);
    let (first, release_first) = blocked(&rt);
    let (entered, started) = mpsc::channel();
    let (release_second, released_second) = mpsc::channel();
    let second = rt.spawn(async move {
        blocking(|| {
            entered.send(()).unwrap();
            released_second.recv().unwrap();
        });
    });
    scan(&rt);
    started.recv_timeout(TIMEOUT).unwrap();
    let (done, completion) = mpsc::channel();
    let third = rt.spawn(async move {
        done.send(()).unwrap();
    });
    scan(&rt);
    let metrics = rt.metrics();
    assert_eq!(metrics.spawned_threads, 2);
    assert_eq!(metrics.handoffs, 1);
    assert!(metrics.capacity_delays > 0);
    assert!(completion.try_recv().is_err());
    release_second.send(()).unwrap();
    completion.recv_timeout(TIMEOUT).unwrap();
    release_first.send(()).unwrap();
    for task in [first, second, third] {
        futures::executor::block_on(task).unwrap();
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn spawn_failure_retains_permit_and_can_recover() {
    let rt = runtime(3);
    let (first, release_first) = blocked(&rt);
    let (entered, started) = mpsc::channel();
    let (release_second, released_second) = mpsc::channel();
    let second = rt.spawn(async move {
        blocking(|| {
            entered.send(()).unwrap();
            released_second.recv().unwrap();
        });
    });
    scan(&rt);
    started.recv_timeout(TIMEOUT).unwrap();
    let (done, completion) = mpsc::channel();
    let third = rt.spawn(async move {
        done.send(()).unwrap();
    });
    rt.shared.fail_spawn.store(true, Ordering::Relaxed);
    scan(&rt);
    assert_eq!(rt.metrics().handoffs, 1);
    assert_eq!(rt.metrics().thread_spawn_failures, 1);
    assert!(completion.try_recv().is_err());
    rt.shared.fail_spawn.store(false, Ordering::Relaxed);
    scan(&rt);
    completion.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(rt.metrics().handoffs, 2);
    assert_eq!(rt.metrics().spawned_threads, 3);
    release_first.send(()).unwrap();
    release_second.send(()).unwrap();
    for task in [first, second, third] {
        futures::executor::block_on(task).unwrap();
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn returning_callers_reacquire_permits_in_fifo_order() {
    let rt = runtime(3);
    let (order_tx, order_rx) = mpsc::channel();
    let (started_tx, started_rx) = mpsc::channel();
    let (release_a, a_rx) = mpsc::channel();
    let a_order = order_tx.clone();
    let a_started = started_tx.clone();
    let a = rt.spawn(async move {
        blocking(|| {
            a_started.send(()).unwrap();
            a_rx.recv().unwrap();
        });
        a_order.send(1).unwrap();
    });
    started_rx.recv_timeout(TIMEOUT).unwrap();
    let (release_b, b_rx) = mpsc::channel();
    let b = rt.spawn(async move {
        blocking(|| {
            started_tx.send(()).unwrap();
            b_rx.recv().unwrap();
        });
        order_tx.send(2).unwrap();
    });
    scan(&rt);
    started_rx.recv_timeout(TIMEOUT).unwrap();
    let (held_tx, held_rx) = mpsc::channel();
    let (release_holder, holder_rx) = mpsc::channel();
    // Deliberately unannotated: this test controls a thread retaining its permit.
    let holder = rt.spawn(async move {
        held_tx.send(()).unwrap();
        holder_rx.recv().unwrap();
    });
    scan(&rt);
    held_rx.recv_timeout(TIMEOUT).unwrap();
    release_a.send(()).unwrap();
    wait_state(&rt, |state| state.returning.len() == 1);
    release_b.send(()).unwrap();
    wait_state(&rt, |state| state.returning.len() == 2);
    release_holder.send(()).unwrap();
    assert_eq!(order_rx.recv_timeout(TIMEOUT).unwrap(), 1);
    assert_eq!(order_rx.recv_timeout(TIMEOUT).unwrap(), 2);
    for task in [a, b, holder] {
        futures::executor::block_on(task).unwrap();
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn shutdown_wakes_pending_tasks_while_a_syscall_is_still_running() {
    let rt = runtime(2);
    let handle = rt.handle();
    let (first, release) = blocked(&rt);
    let pending = rt.spawn(std::future::pending::<()>());
    scan(&rt);
    wait_state(&rt, |state| state.polls > 0);
    assert!(!rt.shutdown_timeout(Duration::ZERO));
    assert!(futures::executor::block_on(pending)
        .unwrap_err()
        .is_cancelled());
    {
        let state = handle.shared.state.lock().unwrap();
        let (state, _) = handle
            .shared
            .changed
            .wait_timeout_while(state, TIMEOUT, |s| s.tasks.len() != 1)
            .unwrap();
        assert_eq!(state.tasks.len(), 1);
    }
    release.send(()).unwrap();
    let _ = futures::executor::block_on(first);
    let state = handle.shared.state.lock().unwrap();
    let (state, _) = handle
        .shared
        .changed
        .wait_timeout_while(state, TIMEOUT, |s| s.live_workers != 0 || s.monitor_alive)
        .unwrap();
    assert_eq!(state.live_workers, 0);
    assert!(!state.monitor_alive);
    assert_eq!(state.permits, 0);
    assert!(state.tasks.is_empty());
}

#[test]
fn panic_after_handoff_restores_execution_capacity() {
    let rt = runtime(2);
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let task = rt.spawn(async move {
        blocking(|| {
            entered.send(()).unwrap();
            released.recv().unwrap();
            panic!("blocking panic");
        });
    });
    started.recv_timeout(TIMEOUT).unwrap();
    let other = rt.spawn(async { 42 });
    scan(&rt);
    assert_eq!(futures::executor::block_on(other).unwrap(), 42);
    release.send(()).unwrap();
    assert!(futures::executor::block_on(task).unwrap_err().is_panic());
    assert_eq!(rt.block_on(async { blocking(|| 9) }), 9);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn fast_returns_and_new_generations_invalidate_monitor_observations() {
    let rt = runtime(2);
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let task = rt.spawn(async move {
        for phase in 0..2 {
            blocking(|| {
                entered.send(phase).unwrap();
                released.recv().unwrap();
            });
        }
        // Hold the original permit after both fast returns to observe it.
        entered.send(2).unwrap();
        released.recv().unwrap();
    });
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), 0);
    let other = rt.spawn(async { 7 });
    let now = Instant::now();
    let mut observations = Vec::new();
    {
        let mut state = rt.shared.state.lock().unwrap();
        rt.shared.scan(&mut state, &mut observations, now);
    }
    release.send(()).unwrap();
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), 1);
    {
        let mut state = rt.shared.state.lock().unwrap();
        // The old observation is expired, but this is a new syscall generation.
        rt.shared
            .scan(&mut state, &mut observations, now + rt.shared.handoff_delay);
        assert_eq!(state.handoffs, 0);
        assert_eq!(state.permits, 1);
    }
    release.send(()).unwrap();
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), 2);
    {
        let mut state = rt.shared.state.lock().unwrap();
        rt.shared.scan(
            &mut state,
            &mut observations,
            now + rt.shared.handoff_delay * 2,
        );
        assert_eq!(state.handoffs, 0);
        assert_eq!(state.permits, 1);
        assert_eq!(state.ready.len(), 1);
    }
    release.send(()).unwrap();
    futures::executor::block_on(task).unwrap();
    assert_eq!(futures::executor::block_on(other).unwrap(), 7);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn racing_return_scan_and_wakes_never_repoll_or_leak_a_permit() {
    let rt = runtime(2);
    for _ in 0..64 {
        let (entered, started) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let mut polled = false;
        let task = rt.spawn(poll_fn(move |cx| {
            assert!(!polled, "a completed future was polled again");
            polled = true;
            blocking(|| {
                entered.send(cx.waker().clone()).unwrap();
                released.recv().unwrap();
            });
            Poll::Ready(())
        }));
        let waker = started.recv_timeout(TIMEOUT).unwrap();
        let other = rt.spawn(async {});
        let start = std::sync::Barrier::new(2);
        thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                for _ in 0..8 {
                    waker.wake_by_ref();
                }
                release.send(()).unwrap();
            });
            start.wait();
            scan(&rt);
        });
        futures::executor::block_on(task).unwrap();
        futures::executor::block_on(other).unwrap();
        wait_state(&rt, |state| state.polling == 0 && state.ready.is_empty());
        assert_eq!(rt.metrics().active_permits, 0);
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn timed_shutdown_waits_for_native_thread_local_destructors() {
    struct ThreadLocalDrop(mpsc::Sender<()>, mpsc::Receiver<()>);
    impl Drop for ThreadLocalDrop {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
            self.1.recv_timeout(TIMEOUT).unwrap();
        }
    }
    thread_local! {
        static ON_EXIT: RefCell<Option<ThreadLocalDrop>> = const { RefCell::new(None) };
    }
    let rt = runtime(2);
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    rt.block_on(async move {
        ON_EXIT.with(|slot| *slot.borrow_mut() = Some(ThreadLocalDrop(entered, released)));
    });
    let handle = rt.handle();
    // Even after all polls return, a native thread may still own TLS data.
    assert!(!rt.shutdown_timeout(Duration::ZERO));
    started.recv_timeout(TIMEOUT).unwrap();
    assert!(handle.metrics().live_threads > 0);
    assert!(handle.shared.state.lock().unwrap().monitor_alive);
    release.send(()).unwrap();
    let state = handle.shared.state.lock().unwrap();
    let (state, _) = handle
        .shared
        .changed
        .wait_timeout_while(state, TIMEOUT, |s| s.monitor_alive)
        .unwrap();
    assert!(!state.monitor_alive);
    assert_eq!(state.live_workers, 0);
}
