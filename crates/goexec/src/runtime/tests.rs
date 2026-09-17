use std::sync::{atomic::Ordering, mpsc};

use super::*;

const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn a_spare_steals_local_children_from_a_blocked_worker() {
    let rt = Runtime::builder()
        .parallelism(1)
        .max_threads(2)
        .build()
        .unwrap();
    rt.block_on(async {
        let (sent, received) = mpsc::channel();
        let child = spawn(async move {
            sent.send(42).unwrap();
        });
        // The child is only in this worker's local queue. The spare must
        // discover it when the monitor detaches the parent's blocking call.
        assert_eq!(blocking(|| received.recv_timeout(TIMEOUT).unwrap()), 42);
        child.await.unwrap();
    });
    assert!(rt.metrics().handoffs >= 1);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn stealing_nested_children_and_blocking_their_owners_keeps_progress() {
    let rt = Runtime::builder()
        .parallelism(4)
        .max_threads(64)
        .build()
        .unwrap();
    rt.block_on(async {
        for _ in 0..8 {
            let parents: Vec<_> = (0..16)
                .map(|_| {
                    spawn(async {
                        let (sent, received) = mpsc::channel();
                        let child = spawn(async move {
                            // Both generations originate in local queues and may be
                            // moved by fast batch steals before their owner blocks.
                            let (sent_again, received_again) = mpsc::channel();
                            let grandchild = spawn(async move {
                                sent_again.send(7).unwrap();
                            });
                            let value = blocking(|| received_again.recv_timeout(TIMEOUT).unwrap());
                            grandchild.await.unwrap();
                            sent.send(value).unwrap();
                        });
                        assert_eq!(blocking(|| received.recv_timeout(TIMEOUT).unwrap()), 7);
                        child.await.unwrap();
                    })
                })
                .collect();
            for parent in parents {
                parent.await.unwrap();
            }
        }
    });
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn external_injection_is_not_starved_by_a_self_waking_local_task() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (started, running) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let done = stop.clone();
    let busy = rt.spawn(async move {
        // Exercise repeated local wakes and scheduler checkpoints before
        // injecting work from outside the runtime.
        for _ in 0..256 {
            yield_now().await;
        }
        started.send(()).unwrap();
        resumed.recv_timeout(TIMEOUT).unwrap();
        for _ in 0..16 {
            yield_now().await;
            if done.load(Ordering::Acquire) {
                return;
            }
        }
        panic!("external work missed the 16-poll injector check");
    });
    running.recv_timeout(TIMEOUT).unwrap();
    let (sent, received) = mpsc::channel();
    let injected = rt.spawn(async move {
        stop.store(true, Ordering::Release);
        sent.send(()).unwrap();
    });
    resume.send(()).unwrap();
    received.recv_timeout(TIMEOUT).unwrap();
    futures::executor::block_on(injected).unwrap();
    futures::executor::block_on(busy).unwrap();
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn self_waking_local_task_does_not_starve_an_older_sibling() {
    let rt = Runtime::builder().parallelism(2).build().unwrap();
    let (started, running) = mpsc::channel();
    let (resume, resumed) = mpsc::channel();
    let occupied = rt.spawn(async move {
        started.send(()).unwrap();
        // Deliberately retain the other permit, so another worker cannot
        // rescue the older task and conceal starvation in the LIFO owner.
        resumed.recv_timeout(TIMEOUT).unwrap();
    });
    running.recv_timeout(TIMEOUT).unwrap();
    rt.block_on(async {
        let ran = Arc::new(AtomicBool::new(false));
        let completed = ran.clone();
        let older = spawn(async move {
            completed.store(true, Ordering::Release);
        });
        for _ in 0..16 {
            yield_now().await;
            if ran.load(Ordering::Acquire) {
                break;
            }
        }
        assert!(ran.load(Ordering::Acquire), "older local work was starved");
        older.await.unwrap();
    });
    resume.send(()).unwrap();
    futures::executor::block_on(occupied).unwrap();
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn shutdown_cancels_a_continuously_runnable_task() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let handle = rt.handle();
    let (started, running) = mpsc::channel();
    let task = rt.spawn(async move {
        for _ in 0..256 {
            yield_now().await;
        }
        started.send(()).unwrap();
        loop {
            yield_now().await;
        }
    });
    running.recv_timeout(TIMEOUT).unwrap();
    // Poll metrics remain visible while the task keeps rescheduling itself.
    assert!(handle.metrics().polls >= 256);
    assert!(rt.shutdown_timeout(TIMEOUT));
    assert!(futures::executor::block_on(task)
        .unwrap_err()
        .is_cancelled());
    assert_eq!(handle.metrics().active_permits, 0);
    assert_eq!(handle.metrics().tasks, 0);
}

#[test]
fn shutdown_drains_local_children_while_their_parent_is_still_blocked() {
    struct CountDrop(Arc<AtomicUsize>);
    impl Drop for CountDrop {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let rt = Runtime::builder()
        .parallelism(1)
        .max_threads(2)
        .build()
        .unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let count = drops.clone();
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mut parent = rt.spawn(async move {
        let mut children = Vec::new();
        for _ in 0..256 {
            let guard = CountDrop(count.clone());
            children.push(spawn(async move {
                let _guard = guard;
                std::future::pending::<()>().await;
            }));
        }
        blocking(|| {
            entered.send(children).unwrap();
            released.recv_timeout(TIMEOUT).unwrap();
        });
    });
    let children = started.recv_timeout(TIMEOUT).unwrap();
    assert!(!rt.shutdown_timeout(Duration::ZERO));
    for child in children {
        assert!(futures::executor::block_on(child)
            .unwrap_err()
            .is_cancelled());
    }
    assert_eq!(drops.load(Ordering::Relaxed), 256);
    assert!(futures::FutureExt::now_or_never(&mut parent).is_none());
    release.send(()).unwrap();
    let _ = futures::executor::block_on(parent);
}

#[test]
fn a_returning_caller_interrupts_a_worker_lease_at_the_poll_boundary() {
    let rt = runtime(2);
    let returned = Arc::new(AtomicBool::new(false));
    let flag = returned.clone();
    let (entered, started) = mpsc::channel();
    let (release_parent, parent_release) = mpsc::channel();
    let parent = rt.spawn(async move {
        blocking(|| {
            entered.send(()).unwrap();
            parent_release.recv_timeout(TIMEOUT).unwrap();
        });
        flag.store(true, Ordering::Release);
    });
    started.recv_timeout(TIMEOUT).unwrap();
    let (polling, polled) = mpsc::channel();
    let (release_poll, poll_release) = mpsc::channel();
    let mut warmup_polls = 0;
    let mut first = true;
    let busy = rt.spawn(poll_fn(move |cx| {
        if warmup_polls < 256 {
            warmup_polls += 1;
            cx.waker().wake_by_ref();
            return Poll::Pending;
        }
        if first {
            first = false;
            polling.send(()).unwrap();
            poll_release.recv_timeout(TIMEOUT).unwrap();
            cx.waker().wake_by_ref();
            Poll::Pending
        } else {
            assert!(
                returned.load(Ordering::Acquire),
                "lease bypassed returning caller"
            );
            Poll::Ready(())
        }
    }));
    scan(&rt);
    polled.recv_timeout(TIMEOUT).unwrap();
    release_parent.send(()).unwrap();
    wait_state(&rt, |state| state.returning.len() == 1);
    release_poll.send(()).unwrap();
    futures::executor::block_on(parent).unwrap();
    futures::executor::block_on(busy).unwrap();
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn successive_enqueues_wake_idle_workers_with_spare_capacity() {
    let mut builder = Runtime::builder().parallelism(2).max_threads(3);
    // An automatic monitor could rearm notifications and hide the lost wake.
    builder.manual_monitor = true;
    let rt = builder.build().unwrap();
    let deadline = Instant::now() + TIMEOUT;
    while rt.shared.state.lock().sleeping_workers != 3 {
        assert!(Instant::now() < deadline, "workers did not park");
        thread::yield_now();
    }

    let (entered, started) = mpsc::channel();
    let mut held = Vec::new();
    let mut releases = Vec::new();
    for _ in 0..2 {
        let entered = entered.clone();
        let (release, released) = mpsc::channel();
        releases.push(release);
        held.push(rt.spawn(async move {
            entered.send(()).unwrap();
            // Keep this permit until both separately enqueued tasks start.
            // The second enqueue must wake a worker that is already parked.
            released.recv().unwrap();
        }));
        started.recv_timeout(TIMEOUT).unwrap();
    }
    for release in releases {
        release.send(()).unwrap();
    }
    for handle in held {
        futures::executor::block_on(handle).unwrap();
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn targeted_wakes_fill_capacity_and_a_single_worker_can_drain_a_burst() {
    let rt = Runtime::builder()
        .parallelism(4)
        .max_threads(8)
        .build()
        .unwrap();
    for _ in 0..16 {
        let (entered, started) = mpsc::channel();
        let mut held = Vec::new();
        let mut releases = Vec::new();
        for _ in 0..4 {
            let entered = entered.clone();
            let (release, released) = mpsc::channel();
            releases.push(release);
            held.push(rt.spawn(async move {
                entered.send(()).unwrap();
                // Deliberately retain the permit until the test releases it.
                released.recv_timeout(TIMEOUT).unwrap();
            }));
        }
        // One notification must cascade to all four available execution slots.
        for _ in 0..4 {
            started.recv_timeout(TIMEOUT).unwrap();
        }
        let (done, completed) = mpsc::channel();
        let mut burst = Vec::new();
        for i in 0..128 {
            let done = done.clone();
            burst.push(rt.spawn(async move {
                yield_now().await;
                done.send(i).unwrap();
            }));
        }
        releases.remove(0).send(()).unwrap();
        let mut result = (0..128)
            .map(|_| completed.recv_timeout(TIMEOUT).unwrap())
            .collect::<Vec<_>>();
        result.sort_unstable();
        assert_eq!(result, (0..128).collect::<Vec<_>>());
        for release in releases {
            release.send(()).unwrap();
        }
        for handle in held.into_iter().chain(burst) {
            futures::executor::block_on(handle).unwrap();
        }
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn external_wake_racing_the_last_poll_keeps_the_runtime_live() {
    let rt = Runtime::builder().parallelism(4).build().unwrap();
    for _ in 0..128 {
        let (started, ready) = mpsc::channel();
        let (done, completed) = mpsc::channel();
        let mut first = true;
        let handle = rt.spawn(poll_fn(move |cx| {
            if first {
                first = false;
                started.send(cx.waker().clone()).unwrap();
                Poll::Pending
            } else {
                done.send(()).unwrap();
                Poll::Ready(())
            }
        }));
        ready.recv_timeout(TIMEOUT).unwrap().wake();
        completed.recv_timeout(TIMEOUT).unwrap();
        futures::executor::block_on(handle).unwrap();
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
}

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
    let mut state = rt.shared.state.lock();
    let now = Instant::now();
    rt.shared.scan(&mut state, &mut observations, now);
    rt.shared
        .scan(&mut state, &mut observations, now + rt.shared.handoff_delay);
}

#[test]
fn existing_workers_can_steal_from_a_newly_added_worker() {
    let rt = runtime(3);
    // Both initial workers take a turn before the third worker exists, so
    // their initial victim lists contain only each other.
    let (first, release_first) = blocked(&rt);
    let (entered_second, second_started) = mpsc::channel();
    let (release_second, second_release) = mpsc::channel();
    let second = rt.spawn(async move {
        blocking(|| {
            entered_second.send(()).unwrap();
            second_release.recv_timeout(TIMEOUT).unwrap();
        });
    });
    scan(&rt);
    second_started.recv_timeout(TIMEOUT).unwrap();

    let (entered_third, third_started) = mpsc::channel();
    let (release_third, third_release) = mpsc::channel();
    let (stolen, child_ran) = mpsc::channel();
    let third = rt.spawn(async move {
        assert_eq!(CURRENT.with(|c| c.borrow().as_ref().unwrap().worker.id), 2);
        let child = spawn(async move {
            stolen
                .send(CURRENT.with(|c| c.borrow().as_ref().unwrap().worker.id))
                .unwrap();
        });
        blocking(|| {
            entered_third.send(()).unwrap();
            third_release.recv_timeout(TIMEOUT).unwrap();
        });
        child.await.unwrap();
    });
    scan(&rt);
    third_started.recv_timeout(TIMEOUT).unwrap();
    assert_eq!(rt.metrics().spawned_threads, 3);

    release_first.send(()).unwrap();
    wait_state(&rt, |state| state.returning.len() == 1);
    scan(&rt);
    // The only runnable is on worker 2; one of the initial workers must find
    // it after refreshing its victim list. The other two remain in syscalls.
    assert!(child_ran.recv_timeout(TIMEOUT).unwrap() < 2);
    release_second.send(()).unwrap();
    release_third.send(()).unwrap();
    futures::executor::block_on(async {
        first.await.unwrap();
        second.await.unwrap();
        third.await.unwrap();
    });
    assert!(rt.shutdown_timeout(TIMEOUT));
}

fn wait_state(rt: &Runtime, predicate: impl Fn(&Scheduler) -> bool) {
    let mut state = rt.shared.state.lock();
    rt.shared
        .changed
        .wait_while_for(&mut state, |s| !predicate(s), TIMEOUT);
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
        let mut state = rt.shared.state.lock();
        rt.shared.scan(&mut state, &mut observations, now);
        rt.shared.scan(&mut state, &mut observations, now);
    }
    assert_eq!(rt.metrics().handoffs, 0);
    assert!(completion.try_recv().is_err());
    {
        let mut state = rt.shared.state.lock();
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
        let mut state = handle.shared.state.lock();
        handle.shared.changed.wait_while_for(
            &mut state,
            |_| handle.shared.task_count() != 1,
            TIMEOUT,
        );
        assert_eq!(handle.shared.task_count(), 1);
    }
    release.send(()).unwrap();
    let _ = futures::executor::block_on(first);
    let mut state = handle.shared.state.lock();
    handle.shared.changed.wait_while_for(
        &mut state,
        |s| s.live_workers != 0 || s.monitor_alive,
        TIMEOUT,
    );
    assert_eq!(state.live_workers, 0);
    assert!(!state.monitor_alive);
    assert_eq!(state.permits, 0);
    assert_eq!(handle.shared.task_count(), 0);
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
        let mut state = rt.shared.state.lock();
        rt.shared.scan(&mut state, &mut observations, now);
    }
    release.send(()).unwrap();
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), 1);
    {
        let mut state = rt.shared.state.lock();
        // The old observation is expired, but this is a new syscall generation.
        rt.shared
            .scan(&mut state, &mut observations, now + rt.shared.handoff_delay);
        assert_eq!(state.handoffs, 0);
        assert_eq!(state.permits, 1);
    }
    release.send(()).unwrap();
    assert_eq!(started.recv_timeout(TIMEOUT).unwrap(), 2);
    {
        let mut state = rt.shared.state.lock();
        rt.shared.scan(
            &mut state,
            &mut observations,
            now + rt.shared.handoff_delay * 2,
        );
        assert_eq!(state.handoffs, 0);
        assert_eq!(state.permits, 1);
        assert!(rt.shared.has_ready(&state));
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
        wait_state(&rt, |state| {
            state.polling == 0 && !rt.shared.has_ready(state)
        });
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
    assert!(handle.shared.state.lock().monitor_alive);
    release.send(()).unwrap();
    let mut state = handle.shared.state.lock();
    handle
        .shared
        .changed
        .wait_while_for(&mut state, |s| s.monitor_alive, TIMEOUT);
    assert!(!state.monitor_alive);
    assert_eq!(state.live_workers, 0);
}
