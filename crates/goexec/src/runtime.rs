use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    fmt,
    future::Future,
    io,
    marker::PhantomData,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    pin::pin,
    rc::Rc,
    sync::{
        atomic::{fence, AtomicBool, AtomicU64, AtomicUsize, Ordering},
        Arc,
    },
    task::{Context, Poll},
    thread,
    time::{Duration, Instant},
};

use async_task::Runnable;
use crossbeam_deque::{Injector, Steal, Stealer, Worker as LocalQueue};
use event_listener::{Event, Listener};
use futures::{
    channel::oneshot,
    future::{poll_fn, AbortHandle, Abortable},
    task::{waker_ref, ArcWake},
};
use parking_lot::{Condvar, Mutex};

use crate::{
    state::CallState,
    task::{JoinError, JoinHandle, TaskFuture},
};

thread_local! {
    static ENTERED: RefCell<Vec<Handle>> = const { RefCell::new(Vec::new()) };
    static CURRENT: RefCell<Option<WorkerContext>> = const { RefCell::new(None) };
}

struct WorkerContext {
    shared: Arc<Shared>,
    worker: Arc<Worker>,
    depth: Cell<usize>,
    local: LocalQueue<Runnable>,
}

struct Worker {
    id: usize,
    call: CallState,
    blocking_started: AtomicU64,
    returned: Condvar,
    stealer: Stealer<Runnable>,
    polls: AtomicU64,
}

struct WorkerEntry {
    worker: Arc<Worker>,
    busy: bool,
}

/// Configure a runtime. There is always at least one spare worker at startup.
#[derive(Clone)]
pub struct Builder {
    parallelism: usize,
    max_threads: Option<usize>,
    handoff_delay: Duration,
    hooks: ThreadHooks,
    #[cfg(test)]
    manual_monitor: bool,
}

#[derive(Clone, Default)]
struct ThreadHooks {
    start: Option<Arc<dyn Fn() + Send + Sync>>,
    park: Option<Arc<dyn Fn() + Send + Sync>>,
    stop: Option<Arc<dyn Fn() + Send + Sync>>,
}

impl fmt::Debug for Builder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Builder")
            .field("parallelism", &self.parallelism)
            .field("max_threads", &self.max_threads)
            .field("handoff_delay", &self.handoff_delay)
            .finish_non_exhaustive()
    }
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            parallelism: thread::available_parallelism().map_or(1, usize::from),
            max_threads: None,
            handoff_delay: Duration::from_micros(100),
            hooks: ThreadHooks::default(),
            #[cfg(test)]
            manual_monitor: false,
        }
    }
}

impl Builder {
    /// Maximum number of workers executing user code at once. Must be nonzero.
    pub fn parallelism(mut self, parallelism: usize) -> Self {
        self.parallelism = parallelism;
        self
    }

    /// Limit OS worker threads, including blocked and spare workers but
    /// excluding the monitor. Must exceed `parallelism`. Defaults to
    /// `max(512, parallelism
    /// + 1)`. At capacity, queued work may wait for an existing call to finish.
    pub fn max_threads(mut self, max_threads: usize) -> Self {
        self.max_threads = Some(max_threads);
        self
    }

    /// Minimum elapsed time in an outermost blocking call before handoff is
    /// eligible. Also sets the monitor's inspection interval. Must be
    /// nonzero; OS scheduling means this is not a real-time deadline.
    pub fn handoff_delay(mut self, delay: Duration) -> Self {
        self.handoff_delay = delay;
        self
    }

    /// Run a callback when each worker starts, with its runtime context
    /// installed. Hooks run without scheduler locks. A hook panic closes
    /// the runtime and requests task cancellation, rather than abandoning a
    /// worker's bookkeeping.
    pub fn on_thread_start(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.hooks.start = Some(Arc::new(hook));
        self
    }

    /// Run a callback before an idle worker may park. Work is checked again
    /// after the callback, so the callback does not necessarily precede a
    /// sleep. This hook does not run around user-marked blocking calls.
    pub fn on_thread_park(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.hooks.park = Some(Arc::new(hook));
        self
    }

    /// Run a callback after a worker has stopped executing tasks and cleared
    /// its runtime context. Shutdown waits for this hook and TLS destructors.
    pub fn on_thread_stop(mut self, hook: impl Fn() + Send + Sync + 'static) -> Self {
        self.hooks.stop = Some(Arc::new(hook));
        self
    }

    /// Start the workers and monitor. Returns an error for invalid
    /// configuration or a startup thread-creation failure.
    pub fn build(self) -> io::Result<Runtime> {
        let initial = self.parallelism.checked_add(1).ok_or_else(invalid_config)?;
        let max_threads = self.max_threads.unwrap_or_else(|| initial.max(512));
        if self.parallelism == 0 || max_threads < initial || self.handoff_delay.is_zero() {
            return Err(invalid_config());
        }
        let shared = Arc::new(Shared {
            clock_origin: Instant::now(),
            parallelism: self.parallelism,
            max_threads,
            handoff_delay: self.handoff_delay,
            hooks: self.hooks,
            state: Mutex::new(Scheduler::default()),
            ready: Injector::new(),
            registry: (0..self.parallelism)
                .map(|_| Mutex::new(Registry::default()))
                .collect(),
            next_registry: AtomicUsize::new(0),
            closed: AtomicBool::new(false),
            returning: AtomicBool::new(false),
            wake_needed: AtomicBool::new(false),
            monitor_parked: AtomicBool::new(false),
            changed: Condvar::new(),
            work_available: Event::new(),
            monitor_wake: Event::new(),
            #[cfg(test)]
            manual_monitor: self.manual_monitor,
            #[cfg(test)]
            fail_spawn: std::sync::atomic::AtomicBool::new(false),
        });

        let startup = (|| {
            let mut state = shared.state.lock();
            for _ in 0..initial {
                shared.start_worker(&mut state)?;
            }
            let monitor = shared.clone();
            state.monitor_alive = true;
            match thread::Builder::new()
                .name("goexec-monitor".into())
                .spawn(move || monitor.monitor())
            {
                Ok(thread) => state.monitor_thread = Some(thread),
                Err(error) => {
                    state.monitor_alive = false;
                    return Err(error);
                }
            }
            Ok(())
        })();
        if let Err(error) = startup {
            shared.close();
            return Err(error);
        }
        Ok(Runtime { shared })
    }
}

fn invalid_config() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        "parallelism and handoff_delay must be nonzero; max_threads must exceed parallelism",
    )
}

/// Owns an executor. Dropping it requests cancellation without waiting.
pub struct Runtime {
    shared: Arc<Shared>,
}

impl Runtime {
    /// Create a builder with default settings.
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Obtain a handle which does not keep the runtime open after its owner
    /// drops.
    pub fn handle(&self) -> Handle {
        Handle {
            shared: self.shared.clone(),
        }
    }

    /// Spawn an independent task. Dropping its handle detaches it.
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.shared.spawn(future)
    }

    /// Submit the root future to a worker and wait for its result.
    ///
    /// Unlike Tokio, the root future and output must be `Send + 'static`. A
    /// root panic is resumed on the caller. Panics if invoked on any goexec
    /// worker or if shutdown cancels the root task.
    pub fn block_on<F>(&self, future: F) -> F::Output
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        assert_outside_worker("block_on");
        match futures::executor::block_on(self.spawn(future)) {
            Ok(output) => output,
            Err(error) if error.is_panic() => resume_unwind(error.into_panic()),
            Err(_) => panic!("goexec root task was cancelled"),
        }
    }

    /// Take a snapshot of runtime counters.
    pub fn metrics(&self) -> Metrics {
        self.shared.metrics()
    }

    /// Cancel tasks and join all workers and the monitor, including their
    /// thread-local destructors. May wait indefinitely for user code to return.
    /// Panics on a goexec worker.
    pub fn shutdown(self) {
        assert_outside_worker("shutdown");
        self.shared.close();
        let mut state = self.shared.state.lock();
        while state.live_workers != 0 || state.monitor_alive {
            self.shared.changed.wait(&mut state);
        }
        let monitor = state.monitor_thread.take();
        drop(state);
        if let Some(monitor) = monitor {
            let _ = monitor.join();
        }
    }

    /// Cancel tasks and wait up to `timeout` for workers and the monitor to
    /// exit. Returns false if a running poll/syscall has not finished.
    /// Remaining threads keep their data alive and clean up when execution
    /// returns. Panics if invoked on a goexec worker.
    pub fn shutdown_timeout(self, timeout: Duration) -> bool {
        assert_outside_worker("shutdown_timeout");
        let start = Instant::now();
        self.shared.close();
        let mut state = self.shared.state.lock();
        while state.live_workers != 0
            || state.monitor_alive
            || state
                .monitor_thread
                .as_ref()
                .is_some_and(|thread| !thread.is_finished())
        {
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return false;
            }
            // A thread cannot notify after running its TLS destructors. Once
            // the monitor is exiting, check its actual completion periodically.
            self.shared
                .changed
                .wait_for(&mut state, remaining.min(Duration::from_millis(1)));
        }
        let monitor = state.monitor_thread.take();
        drop(state);
        if let Some(monitor) = monitor {
            let _ = monitor.join();
        }
        true
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.shared.close();
    }
}

/// A cloneable reference for spawning tasks and reading runtime counters.
#[derive(Clone)]
pub struct Handle {
    shared: Arc<Shared>,
}

impl Handle {
    /// Get the explicitly entered runtime, or the current worker's runtime.
    pub fn try_current() -> Option<Self> {
        ENTERED
            .with(|entered| entered.borrow().last().cloned())
            .or_else(|| {
                CURRENT.with(|current| {
                    current.borrow().as_ref().map(|context| Self {
                        shared: context.shared.clone(),
                    })
                })
            })
    }

    /// Get the current runtime. Panics when no runtime context is available.
    pub fn current() -> Self {
        Self::try_current().expect("no current goexec runtime")
    }

    /// Enter a spawning context on this thread. This does not make the caller
    /// a worker or grant an execution permit. Guards must be dropped in reverse
    /// order and cannot be moved to another thread.
    pub fn enter(&self) -> EnterGuard {
        let depth = ENTERED.with(|entered| {
            let mut entered = entered.borrow_mut();
            entered.push(self.clone());
            entered.len()
        });
        EnterGuard {
            depth,
            not_send: PhantomData,
        }
    }

    /// The configured number of execution permits (not the OS thread count).
    pub fn parallelism(&self) -> usize {
        self.shared.parallelism
    }

    /// Drive a possibly borrowed, non-Send future on the calling thread.
    /// Unlike `Runtime::block_on`, this never moves the future to a worker.
    /// On a worker, only waits between polls are marked blocking, so every
    /// subsequent poll has reacquired the worker's execution permit.
    ///
    /// Nested calls are supported, but not from inside `blocking`: that region
    /// may already have surrendered its permit and must not poll user futures.
    pub fn block_on<F: Future>(&self, future: F) -> F::Output {
        CURRENT.with(|current| {
            if let Some(context) = current.borrow().as_ref() {
                assert_eq!(context.depth.get(), 0, "block_on inside goexec::blocking");
            }
        });
        let _entered = self.enter();
        let parker = Arc::new(Parker::default());
        let waker = waker_ref(&parker);
        let mut cx = Context::from_waker(&waker);
        let mut future = pin!(future);
        loop {
            *parker.notified.lock() = false;
            if let Poll::Ready(value) = future.as_mut().poll(&mut cx) {
                return value;
            }
            blocking(|| {
                let mut notified = parker.notified.lock();
                while !*notified {
                    parker.ready.wait(&mut notified);
                }
            });
        }
    }

    /// Spawn a task. After runtime shutdown, returns an already-cancelled
    /// handle and does not poll the supplied future.
    pub fn spawn<F>(&self, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.shared.spawn(future)
    }

    /// Take a snapshot of runtime counters.
    pub fn metrics(&self) -> Metrics {
        self.shared.metrics()
    }
}

/// A thread-bound runtime context. Drop entered contexts in reverse order.
pub struct EnterGuard {
    depth: usize,
    not_send: PhantomData<Rc<()>>,
}

impl Drop for EnterGuard {
    fn drop(&mut self) {
        ENTERED.with(|entered| {
            let mut entered = entered.borrow_mut();
            assert_eq!(
                entered.len(),
                self.depth,
                "goexec enter guards dropped out of order"
            );
            entered.pop();
        });
    }
}

#[derive(Default)]
struct Parker {
    notified: Mutex<bool>,
    ready: Condvar,
}

impl ArcWake for Parker {
    fn wake_by_ref(this: &Arc<Self>) {
        *this.notified.lock() = true;
        this.ready.notify_one();
    }
}

#[cfg(not(target_family = "wasm"))]
fn wait_listener(listener: impl Listener, timeout: Option<Duration>) {
    match timeout {
        Some(timeout) => {
            listener.wait_timeout(timeout);
        }
        None => listener.wait(),
    }
}

// event-listener exposes blocking waits only on native targets. Threaded WASI
// can still use the same notification protocol through its Future interface.
#[cfg(target_family = "wasm")]
fn wait_listener(listener: impl Listener, timeout: Option<Duration>) {
    wait_listener_portable(listener, timeout);
}

#[cfg(any(all(test, not(feature = "loom")), target_family = "wasm"))]
fn wait_listener_portable(listener: impl Future<Output = ()>, timeout: Option<Duration>) -> bool {
    let parker = Arc::new(Parker::default());
    let waker = waker_ref(&parker);
    let mut cx = Context::from_waker(&waker);
    let mut listener = pin!(listener);
    let deadline = timeout.and_then(|timeout| Instant::now().checked_add(timeout));
    loop {
        *parker.notified.lock() = false;
        if listener.as_mut().poll(&mut cx).is_ready() {
            return true;
        }
        let mut notified = parker.notified.lock();
        while !*notified {
            match deadline {
                Some(deadline) => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return false;
                    }
                    parker.ready.wait_for(&mut notified, remaining);
                }
                None => parker.ready.wait(&mut notified),
            }
        }
    }
}

/// A snapshot. Counts exclude the monitor thread. Queue/task counts are
/// approximate across independently changing queues and registries.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    /// Worker threads not yet joined, including spares and blocked workers.
    pub live_threads: usize,
    /// Successfully created worker threads over this runtime's lifetime.
    pub spawned_threads: usize,
    /// Workers currently owning an execution permit (including unreclaimed
    /// calls).
    pub active_permits: usize,
    /// Runnable tasks waiting in the injection and worker-local queues.
    pub queued_tasks: usize,
    /// Tasks that have not yet been completely destroyed.
    pub tasks: usize,
    /// Calls whose execution permits were handed off.
    pub handoffs: u64,
    /// Eligible handoff attempts prevented by the configured thread limit.
    pub capacity_delays: u64,
    /// Failed attempts to create additional OS worker threads.
    pub thread_spawn_failures: u64,
    /// Completed invocations of a task's poll function.
    pub polls: u64,
}

/// Spawn on the current goexec runtime. Panics without a worker or entered
/// context.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    Handle::current().spawn(future)
}

/// Cooperatively yield to other tasks, once.
pub async fn yield_now() {
    let mut yielded = false;
    poll_fn(move |cx| {
        if yielded {
            Poll::Ready(())
        } else {
            yielded = true;
            cx.waker().wake_by_ref();
            Poll::Pending
        }
    })
    .await
}

/// Execute an explicitly marked blocking operation on the current OS thread.
///
/// Short operations timestamp their start and update thread-local/atomic state.
/// The closure may borrow data and need not be `Send`. Nested calls are
/// accounted for once. Outside goexec this simply calls the closure.
/// CPU-intensive work should not be put in this boundary: compensating for it
/// can oversubscribe CPUs.
pub fn blocking<F: FnOnce() -> R, R>(f: F) -> R {
    CURRENT.with(|current| {
        let current = current.borrow();
        let Some(context) = current.as_ref() else {
            return f();
        };
        let depth = context.depth.get();
        context.depth.set(depth + 1);
        let ticket = (depth == 0).then(|| {
            context.worker.blocking_started.store(
                context.shared.clock_origin.elapsed().as_nanos() as u64,
                Ordering::Relaxed,
            );
            // Release-publish the timestamp with this generation. A monitor
            // racing a later generation cannot detach it using this ticket.
            context.worker.call.enter()
        });
        let _guard = BlockingGuard { context, ticket };
        f()
    })
}

struct BlockingGuard<'a> {
    context: &'a WorkerContext,
    ticket: Option<u64>,
}

impl Drop for BlockingGuard<'_> {
    fn drop(&mut self) {
        self.context.depth.set(self.context.depth.get() - 1);
        if let Some(ticket) = self.ticket {
            if !self.context.worker.call.exit_fast(ticket) {
                self.context
                    .shared
                    .return_from_call(&self.context.worker, ticket);
            }
        }
    }
}

fn assert_outside_worker(operation: &str) {
    CURRENT.with(|current| {
        assert!(
            current.borrow().is_none(),
            "goexec {operation} cannot run on a goexec worker"
        );
    });
}

#[derive(Clone, Copy)]
pub(crate) struct TaskId {
    registry: usize,
    index: u64,
}

#[derive(Default)]
struct Registry {
    tasks: HashMap<u64, AbortHandle>,
    next_id: u64,
}

#[derive(Default)]
struct Scheduler {
    closed: bool,
    workers: Vec<WorkerEntry>,
    threads: Vec<thread::JoinHandle<()>>,
    live_workers: usize,
    monitor_alive: bool,
    monitor_thread: Option<thread::JoinHandle<()>>,
    sleeping_workers: usize,
    permits: usize,
    polling: usize,
    returning: VecDeque<usize>,
    handoffs: u64,
    capacity_delays: u64,
    spawn_failures: u64,
    #[cfg(test)]
    polls: u64,
}

pub(crate) struct Shared {
    clock_origin: Instant,
    parallelism: usize,
    max_threads: usize,
    handoff_delay: Duration,
    hooks: ThreadHooks,
    state: Mutex<Scheduler>,
    ready: Injector<Runnable>,
    registry: Vec<Mutex<Registry>>,
    next_registry: AtomicUsize,
    closed: AtomicBool,
    returning: AtomicBool,
    wake_needed: AtomicBool,
    monitor_parked: AtomicBool,
    changed: Condvar,
    work_available: Event,
    monitor_wake: Event,
    #[cfg(test)]
    manual_monitor: bool,
    #[cfg(test)]
    fail_spawn: AtomicBool,
}

impl Shared {
    fn spawn<F>(self: &Arc<Self>, future: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        let (sender, receiver) = oneshot::channel();
        let (abort, registration) = AbortHandle::new_pair();
        let join = JoinHandle::new(receiver, abort.clone());
        let registry = CURRENT
            .with(|current| {
                current
                    .borrow()
                    .as_ref()
                    .filter(|c| Arc::ptr_eq(&c.shared, self))
                    .map(|c| c.worker.id)
            })
            .unwrap_or_else(|| self.next_registry.fetch_add(1, Ordering::Relaxed))
            % self.registry.len();
        let id = {
            let mut shard = self.registry[registry].lock();
            // Admission and removal share this shard lock. Shutdown publishes
            // closed before checking shards, so registration cannot be missed.
            if self.closed.load(Ordering::Acquire) {
                drop(shard);
                let _ = sender.send(Err(JoinError::cancelled()));
                return join;
            }
            let index = shard.next_id;
            shard.next_id = index.checked_add(1).expect("task ID space exhausted");
            shard.tasks.insert(index, abort);
            TaskId { registry, index }
        };
        let task = TaskFuture::new(
            Abortable::new(future, registration),
            sender,
            self.clone(),
            id,
        );
        let scheduler = self.clone();
        let (runnable, task) = async_task::spawn(task, move |runnable| scheduler.enqueue(runnable));
        task.detach();
        runnable.schedule();
        join
    }

    fn enqueue(&self, runnable: Runnable) {
        CURRENT.with(|current| {
            let current = current.borrow();
            if let Some(context) = current.as_ref().filter(|c| std::ptr::eq(&*c.shared, self)) {
                context.local.push(runnable);
            } else {
                self.ready.push(runnable);
            }
        });
        self.notify_published_work();
    }

    fn notify_published_work(&self) {
        // Pair queue publication with the waiter's signal-before-search fence.
        // No shared counter/RMW is needed when every worker is already busy.
        fence(Ordering::SeqCst);
        if self.wake_needed.load(Ordering::Acquire)
            && self.wake_needed.swap(false, Ordering::AcqRel)
        {
            // Registered listeners retain notifications before their thread
            // actually parks. Publishers therefore need no scheduler lock.
            // The worker still acquires a permit before polling any task.
            self.work_available.notify(1);
            self.wake_monitor();
        }
    }

    pub(crate) fn task_finished(&self, id: TaskId) {
        // AbortHandle's waker can own a task whose destructor reenters us.
        let removed = self.registry[id.registry].lock().tasks.remove(&id.index);
        drop(removed);
        if self.closed.load(Ordering::Acquire) {
            let state = self.state.lock();
            self.notify_shutdown(&state);
        }
    }

    fn task_count(&self) -> usize {
        self.registry.iter().map(|r| r.lock().tasks.len()).sum()
    }

    fn has_ready(&self, state: &Scheduler) -> bool {
        !self.ready.is_empty() || state.workers.iter().any(|w| !w.worker.stealer.is_empty())
    }

    fn finished(&self, state: &Scheduler) -> bool {
        state.closed
            && state.polling == 0
            && !self.has_ready(state)
            && self.registry.iter().all(|r| r.lock().tasks.is_empty())
    }

    fn close(&self) {
        {
            let mut state = self.state.lock();
            if state.closed {
                return;
            }
            self.closed.store(true, Ordering::Release);
            state.closed = true;
        }
        let tasks: Vec<_> = self
            .registry
            .iter()
            .flat_map(|r| r.lock().tasks.values().cloned().collect::<Vec<_>>())
            .collect();
        for task in tasks {
            task.abort();
        }
        self.work_available.notify(usize::MAX);
        self.monitor_wake.notify(1);
        self.changed.notify_all();
    }

    fn wake_available(&self, state: &Scheduler) {
        if state.permits < self.parallelism {
            // One publication can consume the wake request while other workers
            // remain parked. Rearm before checking the queues so a later enqueue
            // either appears in this search or notifies those idle workers.
            self.wake_needed.store(true, Ordering::Release);
            fence(Ordering::SeqCst);
            if let Some(&id) = state.returning.front() {
                state.workers[id].worker.returned.notify_one();
            } else if self.has_ready(state) && state.sleeping_workers != 0 {
                self.work_available.notify(1);
            }
        }
    }

    fn wake_monitor(&self) {
        if self.monitor_parked.load(Ordering::Acquire)
            && self.monitor_parked.swap(false, Ordering::AcqRel)
        {
            self.monitor_wake.notify(1);
        }
    }

    fn notify_shutdown(&self, state: &Scheduler) {
        #[cfg(test)]
        self.changed.notify_all();
        if state.closed {
            self.changed.notify_all();
            self.monitor_wake.notify(1);
            if self.finished(state) {
                self.work_available.notify(usize::MAX);
            }
        }
    }

    fn metrics(&self) -> Metrics {
        let state = self.state.lock();
        Metrics {
            live_threads: state.live_workers,
            spawned_threads: state.workers.len(),
            active_permits: state.permits,
            queued_tasks: self.ready.len()
                + state
                    .workers
                    .iter()
                    .map(|w| w.worker.stealer.len())
                    .sum::<usize>(),
            tasks: self.task_count(),
            handoffs: state.handoffs,
            capacity_delays: state.capacity_delays,
            thread_spawn_failures: state.spawn_failures,
            polls: state
                .workers
                .iter()
                .map(|w| w.worker.polls.load(Ordering::Relaxed))
                .sum(),
        }
    }

    fn start_worker(self: &Arc<Self>, state: &mut Scheduler) -> io::Result<()> {
        #[cfg(test)]
        if self.fail_spawn.load(Ordering::Relaxed) {
            return Err(io::Error::other("injected thread spawn failure"));
        }
        // A single permit benefits from breadth-first progress. With parallel
        // consumers, keep recent work local and let thieves take older work.
        let local = if self.parallelism == 1 {
            LocalQueue::new_fifo()
        } else {
            LocalQueue::new_lifo()
        };
        let worker = Arc::new(Worker {
            id: state.workers.len(),
            call: CallState::new(),
            blocking_started: AtomicU64::new(0),
            returned: Condvar::new(),
            stealer: local.stealer(),
            polls: AtomicU64::new(0),
        });
        let shared = self.clone();
        let runner = worker.clone();
        let thread = thread::Builder::new()
            .name(format!("goexec-worker-{}", worker.id))
            .spawn(move || shared.worker(runner, local))?;
        state.threads.push(thread);
        state.workers.push(WorkerEntry {
            worker,
            busy: false,
        });
        state.live_workers += 1;
        Ok(())
    }

    fn next_runnable(
        &self,
        worker: &Worker,
        local: &LocalQueue<Runnable>,
        stealers: &[Stealer<Runnable>],
        global_first: bool,
        oldest_first: bool,
    ) -> Option<(Runnable, bool)> {
        if global_first {
            if let Steal::Success(task) = self.ready.steal_batch_and_pop(local) {
                return Some((task, true));
            }
        }
        // Recently woken local tasks tend to reuse the worker's hot data. An
        // oldest-first turn prevents a self-waking task from starving siblings.
        // The stealer end remains available to other workers while we block.
        if oldest_first && self.parallelism > 1 {
            if let Steal::Success(task) = worker.stealer.steal() {
                return Some((task, false));
            }
        }
        if let Some(task) = local.pop() {
            return Some((task, false));
        }
        if let Steal::Success(task) = self.ready.steal_batch_and_pop(local) {
            return Some((task, true));
        }
        for stealer in stealers {
            if let Steal::Success(task) = stealer.steal_batch_and_pop(local) {
                return Some((task, true));
            }
        }
        None
    }

    fn worker(self: Arc<Self>, worker: Arc<Worker>, local: LocalQueue<Runnable>) {
        CURRENT.with(|current| {
            *current.borrow_mut() = Some(WorkerContext {
                shared: self.clone(),
                worker: worker.clone(),
                depth: Cell::new(0),
                local,
            });
        });
        self.run_hook(&self.hooks.start);
        CURRENT.with(|current| {
            self.run_worker(&worker, &current.borrow().as_ref().unwrap().local);
        });
        CURRENT.with(|current| {
            current.borrow_mut().take();
        });
        self.run_hook(&self.hooks.stop);
    }

    fn run_hook(&self, hook: &Option<Arc<dyn Fn() + Send + Sync>>) {
        if let Some(hook) = hook {
            if catch_unwind(AssertUnwindSafe(|| hook())).is_err() {
                self.close();
            }
        }
    }

    fn run_worker(&self, worker: &Worker, local: &LocalQueue<Runnable>) {
        let mut stealers = Vec::new();
        let mut state = self.state.lock();
        loop {
            if self.finished(&state) {
                return;
            }
            if state.permits < self.parallelism && state.returning.is_empty() {
                if stealers.len() + 1 != state.workers.len() {
                    stealers = (1..state.workers.len())
                        .map(|offset| {
                            state.workers[(worker.id + offset) % state.workers.len()]
                                .worker
                                .stealer
                                .clone()
                        })
                        .collect();
                }
                if let Some((mut runnable, _)) =
                    self.next_runnable(worker, local, &stealers, true, true)
                {
                    state.permits += 1;
                    state.polling += 1;
                    state.workers[worker.id].busy = true;
                    self.wake_monitor();
                    self.wake_available(&state);
                    drop(state);
                    let mut polls = 0;
                    loop {
                        let _ = catch_unwind(AssertUnwindSafe(|| runnable.run()));
                        worker.polls.fetch_add(1, Ordering::Relaxed);
                        polls += 1;
                        // A permit belongs to this worker across a bounded run
                        // of polls. Returning callers take it at the next poll
                        // boundary; closed admission also forces a checkpoint.
                        if polls == 64
                            || self.returning.load(Ordering::SeqCst)
                            || self.closed.load(Ordering::Acquire)
                        {
                            break;
                        }
                        let Some((next, transferred)) = self.next_runnable(
                            worker,
                            local,
                            &stealers,
                            polls % 16 == 0,
                            polls % 8 == 0,
                        ) else {
                            break;
                        };
                        // A fast batch steal publishes work into a different
                        // deque without holding the scheduler lock. A waiter
                        // may have seen neither queue during the transfer.
                        if transferred {
                            self.notify_published_work();
                        }
                        runnable = next;
                    }
                    state = self.state.lock();
                    state.permits -= 1;
                    state.polling -= 1;
                    #[cfg(test)]
                    {
                        state.polls += polls;
                    }
                    state.workers[worker.id].busy = false;
                    self.notify_shutdown(&state);
                    continue;
                }
            }
            if self.hooks.park.is_some() {
                drop(state);
                self.run_hook(&self.hooks.park);
                state = self.state.lock();
                if self.finished(&state) {
                    return;
                }
            }
            // Register before the final queue check. An enqueue between this
            // check and wait is retained even after dropping the scheduler lock.
            event_listener::listener!(self.work_available => listener);
            self.wake_available(&state);
            state.sleeping_workers += 1;
            if state.permits < self.parallelism || self.monitor_parked.load(Ordering::Acquire) {
                self.wake_needed.store(true, Ordering::Release);
                fence(Ordering::SeqCst);
            }
            // Either the final queue search sees work, or its publisher sees
            // wake_needed and takes this mutex to notify after we park.
            if state.permits < self.parallelism
                && state.returning.is_empty()
                && self.has_ready(&state)
            {
                state.sleeping_workers -= 1;
                drop(state);
                thread::yield_now();
                state = self.state.lock();
                continue;
            }
            drop(state);
            wait_listener(listener, None);
            state = self.state.lock();
            state.sleeping_workers -= 1;
        }
    }

    fn return_from_call(&self, worker: &Worker, ticket: u64) {
        let mut state = self.state.lock();
        state.returning.push_back(worker.id);
        self.returning.store(true, Ordering::SeqCst);
        self.wake_monitor();
        self.notify_shutdown(&state);
        while state.permits == self.parallelism || state.returning.front() != Some(&worker.id) {
            worker.returned.wait(&mut state);
        }
        state.returning.pop_front();
        self.returning
            .store(!state.returning.is_empty(), Ordering::SeqCst);
        state.permits += 1;
        worker.call.resume(ticket);
        self.wake_available(&state);
        self.notify_shutdown(&state);
    }

    fn monitor(self: Arc<Self>) {
        let mut observations = Vec::new();
        let mut state = self.state.lock();
        loop {
            event_listener::listener!(self.monitor_wake => listener);
            if self.finished(&state) {
                let threads = std::mem::take(&mut state.threads);
                drop(state);
                for thread in threads {
                    let _ = thread.join();
                    let mut state = self.state.lock();
                    state.live_workers -= 1;
                    self.changed.notify_all();
                }
                let mut state = self.state.lock();
                state.monitor_alive = false;
                self.changed.notify_all();
                return;
            }
            #[cfg(test)]
            if self.manual_monitor {
                drop(state);
                wait_listener(listener, None);
                state = self.state.lock();
                continue;
            }
            if !self.has_ready(&state) && state.returning.is_empty() {
                observations.clear();
                self.monitor_parked.store(true, Ordering::Release);
                self.wake_needed.store(true, Ordering::Release);
                fence(Ordering::SeqCst);
                if !self.has_ready(&state) {
                    drop(state);
                    wait_listener(listener, None);
                    state = self.state.lock();
                }
                self.monitor_parked.store(false, Ordering::Release);
                continue;
            }
            self.scan(&mut state, &mut observations, Instant::now());
            drop(state);
            wait_listener(listener, Some(self.handoff_delay));
            state = self.state.lock();
        }
    }

    fn scan(
        self: &Arc<Self>,
        state: &mut Scheduler,
        observations: &mut Vec<Option<(u64, Instant)>>,
        now: Instant,
    ) {
        observations.resize(state.workers.len(), None);
        for (index, observation) in observations.iter_mut().enumerate() {
            let worker = state.workers[index].worker.clone();
            let Some(ticket) = worker.call.syscall() else {
                *observation = None;
                continue;
            };
            let since = match *observation {
                Some((previous, since)) if previous == ticket => since,
                _ => {
                    let since = self.clock_origin
                        + Duration::from_nanos(worker.blocking_started.load(Ordering::Relaxed));
                    *observation = Some((ticket, since));
                    since
                }
            };
            if now.saturating_duration_since(since) < self.handoff_delay {
                continue;
            }
            let available = self.parallelism - state.permits;
            let demand = self.ready.len()
                + state
                    .workers
                    .iter()
                    .map(|entry| entry.worker.stealer.len())
                    .sum::<usize>()
                + state.returning.len();
            // A single scan can reclaim several blocked permits. Stop once
            // existing free permits cover the ready work and returning callers.
            if demand <= available {
                continue;
            }
            let replacements =
                state.workers.iter().filter(|entry| !entry.busy).count() + state.returning.len();
            // Reserve enough actual workers for every permit released by this
            // scan, even though none can acquire the scheduler lock yet.
            if replacements <= available {
                if state.live_workers == self.max_threads {
                    state.capacity_delays += 1;
                    continue;
                }
                if self.start_worker(state).is_err() {
                    state.spawn_failures += 1;
                    continue;
                }
            }
            if worker.call.detach(ticket) {
                state.permits -= 1;
                state.handoffs += 1;
                self.wake_available(state);
                self.notify_shutdown(state);
            }
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests;
