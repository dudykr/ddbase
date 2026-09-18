#![cfg(not(feature = "loom"))]

use std::{
    future::Future,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    pin::Pin,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc,
    },
    task::{Context, Poll},
    thread,
    time::Duration,
};

use goexec::{blocking, Runtime};

const TIMEOUT: Duration = Duration::from_secs(5);

#[test]
fn spawn_join_detach_and_cross_runtime_await() {
    let rt = Runtime::builder().parallelism(2).build().unwrap();
    let other = Runtime::builder().parallelism(1).build().unwrap();
    let task = rt.spawn(async { goexec::spawn(async { 21 }).await.unwrap() * 2 });
    assert_eq!(other.block_on(task).unwrap(), 42);
    let (tx, rx) = mpsc::channel();
    drop(rt.spawn(async move {
        tx.send(7).unwrap();
    }));
    assert_eq!(rx.recv_timeout(TIMEOUT).unwrap(), 7);
    assert!(rt.shutdown_timeout(TIMEOUT));
    assert!(other.shutdown_timeout(TIMEOUT));
}

#[test]
fn blocking_can_borrow_non_send_values_and_runs_outside_runtime() {
    let value = std::rc::Rc::new(3);
    assert_eq!(blocking(|| *value), 3);
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    assert_eq!(
        rt.block_on(async {
            let value = std::rc::Rc::new(std::cell::Cell::new(4));
            blocking(|| {
                blocking(|| {
                    value.set(value.get() + 1);
                    value.get()
                })
            })
        }),
        5
    );
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn real_blocking_read_does_not_stop_an_independent_task() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client.set_read_timeout(Some(TIMEOUT)).unwrap();
    let (mut peer, _) = listener.accept().unwrap();
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (entered, started) = mpsc::channel();
    let read = rt.spawn(async move {
        let mut client = client;
        let mut byte = [0];
        blocking(|| {
            entered.send(()).unwrap();
            client.read_exact(&mut byte).unwrap();
        });
        byte[0]
    });
    started.recv_timeout(TIMEOUT).unwrap();
    let writer = rt.spawn(async move {
        blocking(|| peer.write_all(&[42]).unwrap());
    });
    assert_eq!(futures::executor::block_on(read).unwrap(), 42);
    futures::executor::block_on(writer).unwrap();
    assert!(rt.metrics().handoffs >= 1);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

struct Dropped(Arc<AtomicUsize>);
impl Drop for Dropped {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn cancellation_and_shutdown_destroy_pending_futures_once() {
    let rt = Runtime::builder().parallelism(2).build().unwrap();
    let handle = rt.handle();
    let drops = Arc::new(AtomicUsize::new(0));
    let (tx, rx) = mpsc::channel();
    let mut tasks = Vec::new();
    for _ in 0..32 {
        let guard = Dropped(drops.clone());
        let tx = tx.clone();
        tasks.push(rt.spawn(async move {
            let _guard = guard;
            tx.send(()).unwrap();
            std::future::pending::<()>().await;
        }));
    }
    for _ in 0..32 {
        rx.recv_timeout(TIMEOUT).unwrap();
    }
    for task in &tasks[..16] {
        task.abort();
    }
    assert!(rt.shutdown_timeout(TIMEOUT));
    for task in tasks {
        assert!(futures::executor::block_on(task)
            .unwrap_err()
            .is_cancelled());
    }
    assert_eq!(drops.load(Ordering::SeqCst), 32);
    let ran = Arc::new(AtomicBool::new(false));
    let check = ran.clone();
    assert!(futures::executor::block_on(handle.spawn(async move {
        check.store(true, Ordering::SeqCst);
    }))
    .unwrap_err()
    .is_cancelled());
    assert!(!ran.load(Ordering::SeqCst));
}

#[test]
fn abort_before_first_poll_does_not_run_the_future() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (entered, started) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let busy = rt.spawn(async move {
        entered.send(()).unwrap();
        // Deliberately hold the sole permit until the queued task is aborted.
        released.recv_timeout(TIMEOUT).unwrap();
    });
    started.recv_timeout(TIMEOUT).unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let guard = Dropped(drops.clone());
    let task = rt.spawn(async move {
        let _guard = guard;
        panic!("an already-aborted future was polled");
    });
    task.abort();
    release.send(()).unwrap();
    futures::executor::block_on(busy).unwrap();
    assert!(futures::executor::block_on(task)
        .unwrap_err()
        .is_cancelled());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn abort_wakes_a_parked_task_after_many_self_wakes() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let (started, running) = mpsc::channel();
    let (dropped, destroyed) = mpsc::channel();
    struct NotifyDrop(mpsc::Sender<()>);
    impl Drop for NotifyDrop {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
        }
    }
    let task = rt.spawn(async move {
        let _guard = NotifyDrop(dropped);
        for _ in 0..1024 {
            goexec::yield_now().await;
        }
        started.send(()).unwrap();
        std::future::pending::<()>().await;
    });
    running.recv_timeout(TIMEOUT).unwrap();
    // With one permit this sentinel can run only after the task's final poll
    // returned Pending. The task now depends entirely on its abort waker.
    rt.block_on(async {});
    // The original abort waker must survive repeated polls, even when the
    // task stops waking itself.
    task.abort();
    destroyed.recv_timeout(TIMEOUT).unwrap();
    assert!(futures::executor::block_on(task)
        .unwrap_err()
        .is_cancelled());
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn abort_does_not_drop_a_buffer_borrowed_by_a_running_call() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let guard = Dropped(drops.clone());
    let (tx, rx) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let task = rt.spawn(async move {
        let _guard = guard;
        let mut bytes = vec![1; 128];
        blocking(|| {
            tx.send(()).unwrap();
            released.recv().unwrap();
            bytes[0] = 42;
        });
        std::future::pending::<()>().await;
    });
    rx.recv_timeout(TIMEOUT).unwrap();
    task.abort();
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    assert!(futures::executor::block_on(task)
        .unwrap_err()
        .is_cancelled());
    assert!(rt.shutdown_timeout(TIMEOUT));
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn poll_and_drop_panics_are_join_errors() {
    #[derive(Debug)]
    struct PanicOnDrop;
    impl Drop for PanicOnDrop {
        fn drop(&mut self) {
            panic!("drop panic");
        }
    }
    struct DropFuture(PanicOnDrop);
    impl Future for DropFuture {
        type Output = ();

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
            Poll::Ready(())
        }
    }
    struct DoubleDropFuture(PanicOnDrop);
    impl Future for DoubleDropFuture {
        type Output = PanicOnDrop;

        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<PanicOnDrop> {
            Poll::Ready(PanicOnDrop)
        }
    }
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let task = rt.spawn(async {
        panic!("poll panic");
    });
    let error = futures::executor::block_on(task).unwrap_err();
    assert!(error.is_panic());
    assert_eq!(
        *error.into_panic().downcast::<&str>().unwrap(),
        "poll panic"
    );
    assert!(
        futures::executor::block_on(rt.spawn(DropFuture(PanicOnDrop)))
            .unwrap_err()
            .is_panic()
    );
    // These are separate destructor unwinds, not a double panic while unwinding.
    assert!(
        futures::executor::block_on(rt.spawn(DoubleDropFuture(PanicOnDrop)))
            .unwrap_err()
            .is_panic()
    );
    assert_eq!(rt.block_on(async { 11 }), 11);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn root_panics_propagate_and_nested_block_on_is_rejected() {
    let rt = Arc::new(Runtime::builder().parallelism(1).build().unwrap());
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| rt.block_on(async {
            panic!("root");
        })))
        .is_err()
    );
    let nested = rt.clone();
    assert!(futures::executor::block_on(rt.spawn(async move {
        nested.block_on(async {});
    }))
    .unwrap_err()
    .is_panic());
    assert!(Arc::try_unwrap(rt).ok().unwrap().shutdown_timeout(TIMEOUT));
}

struct ManyWakes {
    remaining: usize,
    in_poll: Arc<AtomicBool>,
}
impl Future for ManyWakes {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        assert!(
            !self.in_poll.swap(true, Ordering::SeqCst),
            "concurrent poll"
        );
        let result = if self.remaining == 0 {
            Poll::Ready(())
        } else {
            self.remaining -= 1;
            thread::scope(|scope| {
                for _ in 0..3 {
                    let waker = cx.waker().clone();
                    scope.spawn(move || {
                        for _ in 0..8 {
                            waker.wake_by_ref();
                        }
                    });
                }
            });
            Poll::Pending
        };
        self.in_poll.store(false, Ordering::SeqCst);
        result
    }
}

#[test]
fn concurrent_repeated_wakes_never_poll_a_task_twice() {
    let rt = Runtime::builder().parallelism(4).build().unwrap();
    rt.block_on(ManyWakes {
        remaining: 64,
        in_poll: Arc::new(AtomicBool::new(false)),
    });
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn cpu_execution_respects_parallelism() {
    let rt = Runtime::builder().parallelism(2).build().unwrap();
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();
    for _ in 0..24 {
        let active = active.clone();
        let peak = peak.clone();
        tasks.push(rt.spawn(async move {
            for _ in 0..100 {
                let count = active.fetch_add(1, Ordering::SeqCst) + 1;
                peak.fetch_max(count, Ordering::SeqCst);
                for i in 0..100 {
                    std::hint::black_box(i);
                }
                active.fetch_sub(1, Ordering::SeqCst);
                goexec::yield_now().await;
            }
        }));
    }
    for task in tasks {
        futures::executor::block_on(task).unwrap();
    }
    assert!(peak.load(Ordering::SeqCst) <= 2);
    assert_eq!(rt.metrics().handoffs, 0);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn spawn_and_shutdown_race_is_accounted_for() {
    let rt = Runtime::builder().parallelism(2).build().unwrap();
    let handle = rt.handle();
    let spawner = thread::spawn(move || {
        (0..500)
            .map(|i| {
                handle.spawn(async move {
                    goexec::yield_now().await;
                    i
                })
            })
            .collect::<Vec<_>>()
    });
    assert!(rt.shutdown_timeout(TIMEOUT));
    for task in spawner.join().unwrap() {
        let _ = futures::executor::block_on(task);
    }
}

#[test]
fn invalid_configuration_is_rejected() {
    assert!(Runtime::builder().parallelism(0).build().is_err());
    assert!(Runtime::builder()
        .parallelism(2)
        .max_threads(2)
        .build()
        .is_err());
    assert!(Runtime::builder()
        .handoff_delay(Duration::ZERO)
        .build()
        .is_err());
}

#[test]
fn detached_and_unjoined_output_panics_do_not_escape() {
    struct Output(mpsc::Sender<()>);
    impl Drop for Output {
        fn drop(&mut self) {
            self.0.send(()).unwrap();
            panic!("unobserved output destructor");
        }
    }
    for detach_before_completion in [true, false] {
        let rt = Runtime::builder().parallelism(1).build().unwrap();
        let (release, wait) = futures::channel::oneshot::channel();
        let (dropped, destroyed) = mpsc::channel();
        let task = rt.spawn(async move {
            wait.await.unwrap();
            Output(dropped)
        });
        if detach_before_completion {
            drop(task);
            release.send(()).unwrap();
        } else {
            release.send(()).unwrap();
            // The single permit and FIFO queue put this sentinel after the
            // completed task. Dropping the join handle now destroys its result
            // on this caller instead of on the worker.
            rt.block_on(async {});
            assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| drop(task))).is_ok());
        }
        destroyed.recv_timeout(TIMEOUT).unwrap();
        assert_eq!(rt.block_on(async { 42 }), 42);
        assert!(rt.shutdown_timeout(TIMEOUT));
        assert!(destroyed.try_recv().is_err());
    }
}

#[test]
fn completed_abort_handles_do_not_cancel_reused_registrations() {
    let rt = Runtime::builder().parallelism(1).build().unwrap();
    let mut completed = Vec::new();
    for n in 0..128 {
        let task = rt.spawn(async move { n });
        completed.push(task.abort_handle());
        assert_eq!(futures::executor::block_on(task).unwrap(), n);
    }
    let (release, wait) = futures::channel::oneshot::channel();
    let pending = rt.spawn(async move { wait.await.unwrap() });
    // Ensure the new task has installed its cancellation waker before old
    // cancellation handles are invoked, exercising both token and waker reuse.
    rt.block_on(async {});
    for abort in completed {
        abort.abort();
    }
    release.send(42).unwrap();
    assert_eq!(futures::executor::block_on(pending).unwrap(), 42);
    assert!(rt.shutdown_timeout(TIMEOUT));
}

#[test]
fn join_transfers_a_pinned_output_and_drops_it_once() {
    struct Output {
        drops: Arc<AtomicUsize>,
        _pinned: std::marker::PhantomPinned,
    }
    impl Drop for Output {
        fn drop(&mut self) {
            self.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    let rt = Runtime::builder().parallelism(2).build().unwrap();
    let drops = Arc::new(AtomicUsize::new(0));
    let count = drops.clone();
    let value = futures::executor::block_on(rt.spawn(async move {
        Output {
            drops: count,
            _pinned: std::marker::PhantomPinned,
        }
    }))
    .unwrap();
    assert!(rt.shutdown_timeout(TIMEOUT));
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(value);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[test]
fn small_and_large_futures_stay_pinned_and_are_destroyed_once_after_panics() {
    struct PinnedFuture<const N: usize> {
        address: std::cell::Cell<usize>,
        polls: std::cell::Cell<usize>,
        drops: Arc<AtomicUsize>,
        entered: mpsc::Sender<()>,
        complete: bool,
        poll_panics: bool,
        drop_panics: bool,
        _pin: std::marker::PhantomPinned,
        _padding: [u8; N],
    }

    impl<const N: usize> Future for PinnedFuture<N> {
        type Output = ();

        fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
            let this = self.as_ref().get_ref();
            let address = this as *const Self as usize;
            let polls = this.polls.get();
            if polls == 0 {
                this.address.set(address);
                this.entered.send(()).unwrap();
            } else {
                assert_eq!(this.address.get(), address, "future moved between polls");
            }
            this.polls.set(polls + 1);
            if polls != 0 && this.complete {
                assert!(!this.poll_panics, "poll panic");
                Poll::Ready(())
            } else {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        }
    }

    impl<const N: usize> Drop for PinnedFuture<N> {
        fn drop(&mut self) {
            assert_eq!(
                self.address.get(),
                self as *const Self as usize,
                "future moved before drop"
            );
            assert_eq!(self.drops.fetch_add(1, Ordering::SeqCst), 0);
            assert!(!self.drop_panics, "drop panic");
        }
    }

    fn check<const N: usize>() {
        for complete in [false, true] {
            for poll_panics in [false, true] {
                for drop_panics in [false, true] {
                    let rt = Runtime::builder().parallelism(1).build().unwrap();
                    let drops = Arc::new(AtomicUsize::new(0));
                    let (entered, started) = mpsc::channel();
                    let task = rt.spawn(PinnedFuture {
                        address: std::cell::Cell::new(0),
                        polls: std::cell::Cell::new(0),
                        drops: drops.clone(),
                        entered,
                        complete,
                        poll_panics,
                        drop_panics,
                        _pin: std::marker::PhantomPinned,
                        _padding: [0; N],
                    });
                    started.recv_timeout(TIMEOUT).unwrap();
                    if !complete {
                        task.abort();
                    }
                    let result = futures::executor::block_on(task);
                    if drop_panics || (complete && poll_panics) {
                        assert!(result.unwrap_err().is_panic());
                    } else if complete {
                        result.unwrap();
                    } else {
                        assert!(result.unwrap_err().is_cancelled());
                    }
                    assert!(rt.shutdown_timeout(TIMEOUT));
                    assert_eq!(drops.load(Ordering::SeqCst), 1);
                }
            }
        }
    }
    check::<0>();
    check::<1024>();
}
