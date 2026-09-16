use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, VecDeque},
    future::Future,
    io,
    panic::{catch_unwind, resume_unwind, AssertUnwindSafe},
    sync::{Arc, Condvar, Mutex},
    task::Poll,
    thread,
    time::{Duration, Instant},
};

use async_task::Runnable;
use futures::{
    channel::oneshot,
    future::{poll_fn, AbortHandle, Abortable},
};

use crate::{
    state::CallState,
    task::{JoinError, JoinHandle, TaskFuture},
};

thread_local! {
    static CURRENT: RefCell<Option<WorkerContext>> = const { RefCell::new(None) };
}

struct WorkerContext {
    shared: Arc<Shared>,
    worker: Arc<Worker>,
    depth: Cell<usize>,
}

struct Worker {
    id: usize,
    call: CallState,
}

struct WorkerEntry {
    worker: Arc<Worker>,
    busy: bool,
}

/// Configure a runtime. There is always at least one spare worker at startup.
#[derive(Clone, Debug)]
pub struct Builder {
    parallelism: usize,
    max_threads: Option<usize>,
    handoff_delay: Duration,
    #[cfg(test)]
    manual_monitor: bool,
}

impl Default for Builder {
    fn default() -> Self {
        Self {
            parallelism: thread::available_parallelism().map_or(1, usize::from),
            max_threads: None,
            handoff_delay: Duration::from_micros(100),
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

    /// Time a syscall generation must remain observed before handoff is
    /// eligible. Also sets the monitor's inspection interval. Must be
    /// nonzero; OS scheduling means this is not a real-time deadline.
    pub fn handoff_delay(mut self, delay: Duration) -> Self {
        self.handoff_delay = delay;
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
            parallelism: self.parallelism,
            max_threads,
            handoff_delay: self.handoff_delay,
            state: Mutex::new(Scheduler::default()),
            changed: Condvar::new(),
            #[cfg(test)]
            manual_monitor: self.manual_monitor,
            #[cfg(test)]
            fail_spawn: std::sync::atomic::AtomicBool::new(false),
        });

        let startup = (|| {
            let mut state = shared.state.lock().unwrap();
            for _ in 0..initial {
                shared.start_worker(&mut state)?;
            }
            let monitor = shared.clone();
            state.monitor_alive = true;
            if let Err(error) = thread::Builder::new()
                .name("goexec-monitor".into())
                .spawn(move || monitor.monitor())
            {
                state.monitor_alive = false;
                return Err(error);
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

    /// Cancel tasks and wait up to `timeout` for workers and the monitor to
    /// exit. Returns false if a running poll/syscall has not finished.
    /// Remaining threads keep their data alive and clean up when execution
    /// returns. Panics if invoked on a goexec worker.
    pub fn shutdown_timeout(self, timeout: Duration) -> bool {
        assert_outside_worker("shutdown_timeout");
        let start = Instant::now();
        self.shared.close();
        let mut state = self.shared.state.lock().unwrap();
        while state.live_workers != 0 || state.monitor_alive {
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return false;
            }
            state = self
                .shared
                .changed
                .wait_timeout(state, remaining)
                .unwrap()
                .0;
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

/// A snapshot. Counts exclude the monitor thread.
#[derive(Clone, Copy, Debug, Default)]
pub struct Metrics {
    /// Worker threads not yet joined, including spares and blocked workers.
    pub live_threads: usize,
    /// Successfully created worker threads over this runtime's lifetime.
    pub spawned_threads: usize,
    /// Workers currently owning an execution permit (including unreclaimed
    /// calls).
    pub active_permits: usize,
    /// Runnable tasks waiting in the shared queue.
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

/// Spawn on the current goexec runtime. Panics outside a goexec worker.
pub fn spawn<F>(future: F) -> JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let shared = CURRENT.with(|current| {
        current
            .borrow()
            .as_ref()
            .expect("goexec::spawn called outside a worker")
            .shared
            .clone()
    });
    shared.spawn(future)
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
/// Short operations only update thread-local/atomic state. The closure may
/// borrow data and need not be `Send`. Nested calls are accounted for once.
/// Outside goexec this simply calls the closure. CPU-intensive work should not
/// be put in this boundary: compensating for it can oversubscribe CPUs.
pub fn blocking<F: FnOnce() -> R, R>(f: F) -> R {
    CURRENT.with(|current| {
        let current = current.borrow();
        let Some(context) = current.as_ref() else {
            return f();
        };
        let depth = context.depth.get();
        context.depth.set(depth + 1);
        let ticket = (depth == 0).then(|| context.worker.call.enter());
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

#[derive(Default)]
struct Scheduler {
    ready: VecDeque<Runnable>,
    tasks: HashMap<u64, AbortHandle>,
    next_task: u64,
    closed: bool,
    workers: Vec<WorkerEntry>,
    threads: Vec<thread::JoinHandle<()>>,
    live_workers: usize,
    monitor_alive: bool,
    permits: usize,
    polling: usize,
    returning: VecDeque<usize>,
    handoffs: u64,
    capacity_delays: u64,
    spawn_failures: u64,
    polls: u64,
}

impl Scheduler {
    fn finished(&self) -> bool {
        self.closed && self.tasks.is_empty() && self.ready.is_empty() && self.polling == 0
    }
}

pub(crate) struct Shared {
    parallelism: usize,
    max_threads: usize,
    handoff_delay: Duration,
    state: Mutex<Scheduler>,
    changed: Condvar,
    #[cfg(test)]
    manual_monitor: bool,
    #[cfg(test)]
    fail_spawn: std::sync::atomic::AtomicBool,
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
        let id = {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                drop(state);
                let _ = sender.send(Err(JoinError::cancelled()));
                return join;
            }
            let id = state.next_task;
            state.next_task = state
                .next_task
                .checked_add(1)
                .expect("task ID space exhausted");
            state.tasks.insert(id, abort);
            id
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
        self.state.lock().unwrap().ready.push_back(runnable);
        self.changed.notify_all();
    }

    pub(crate) fn task_finished(&self, id: u64) {
        // Drop AbortHandle outside the scheduler lock: its AtomicWaker may own
        // the last reference to a task whose destructor calls back into us.
        let removed = self.state.lock().unwrap().tasks.remove(&id);
        drop(removed);
        self.changed.notify_all();
    }

    fn close(&self) {
        let tasks = {
            let mut state = self.state.lock().unwrap();
            if state.closed {
                return;
            }
            state.closed = true;
            state.tasks.values().cloned().collect::<Vec<_>>()
        };
        // Abort may synchronously invoke the task's scheduling callback.
        for task in tasks {
            task.abort();
        }
        self.changed.notify_all();
    }

    fn metrics(&self) -> Metrics {
        let state = self.state.lock().unwrap();
        Metrics {
            live_threads: state.live_workers,
            spawned_threads: state.workers.len(),
            active_permits: state.permits,
            queued_tasks: state.ready.len(),
            tasks: state.tasks.len(),
            handoffs: state.handoffs,
            capacity_delays: state.capacity_delays,
            thread_spawn_failures: state.spawn_failures,
            polls: state.polls,
        }
    }

    fn start_worker(self: &Arc<Self>, state: &mut Scheduler) -> io::Result<()> {
        #[cfg(test)]
        if self.fail_spawn.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(io::Error::other("injected thread spawn failure"));
        }
        let worker = Arc::new(Worker {
            id: state.workers.len(),
            call: CallState::new(),
        });
        let shared = self.clone();
        let runner = worker.clone();
        // The new thread cannot take the scheduler mutex until its descriptor
        // and live count have been installed by this caller.
        let thread = thread::Builder::new()
            .name(format!("goexec-worker-{}", worker.id))
            .spawn(move || shared.worker(runner))?;
        state.threads.push(thread);
        state.workers.push(WorkerEntry {
            worker,
            busy: false,
        });
        state.live_workers += 1;
        Ok(())
    }

    fn worker(self: Arc<Self>, worker: Arc<Worker>) {
        CURRENT.with(|current| {
            *current.borrow_mut() = Some(WorkerContext {
                shared: self.clone(),
                worker: worker.clone(),
                depth: Cell::new(0),
            });
        });
        loop {
            let runnable = {
                let mut state = self.state.lock().unwrap();
                loop {
                    if state.finished() {
                        // The monitor joins native threads, including any user
                        // TLS destructors, before publishing shutdown completion.
                        CURRENT.with(|current| {
                            current.borrow_mut().take();
                        });
                        self.changed.notify_all();
                        return;
                    }
                    if state.permits < self.parallelism && state.returning.is_empty() {
                        if let Some(runnable) = state.ready.pop_front() {
                            state.permits += 1;
                            state.polling += 1;
                            state.workers[worker.id].busy = true;
                            break runnable;
                        }
                    }
                    state = self.changed.wait(state).unwrap();
                }
            };
            // TaskFuture catches user poll/drop panics. This final boundary also
            // prevents an unexpected unwind from losing an execution permit.
            let _ = catch_unwind(AssertUnwindSafe(|| runnable.run()));
            let mut state = self.state.lock().unwrap();
            state.permits -= 1;
            state.polling -= 1;
            state.polls += 1;
            state.workers[worker.id].busy = false;
            self.changed.notify_all();
        }
    }

    fn return_from_call(&self, worker: &Worker, ticket: u64) {
        let mut state = self.state.lock().unwrap();
        state.returning.push_back(worker.id);
        self.changed.notify_all();
        while state.permits == self.parallelism || state.returning.front() != Some(&worker.id) {
            state = self.changed.wait(state).unwrap();
        }
        state.returning.pop_front();
        state.permits += 1;
        worker.call.resume(ticket);
        self.changed.notify_all();
    }

    fn monitor(self: Arc<Self>) {
        let mut observations = Vec::new();
        let mut state = self.state.lock().unwrap();
        loop {
            if state.finished() {
                let threads = std::mem::take(&mut state.threads);
                drop(state);
                for thread in threads {
                    // Never join with the scheduler mutex held: other workers
                    // still need it to observe shutdown and leave their loops.
                    let _ = thread.join();
                    let mut state = self.state.lock().unwrap();
                    state.live_workers -= 1;
                    self.changed.notify_all();
                }
                let mut state = self.state.lock().unwrap();
                state.monitor_alive = false;
                self.changed.notify_all();
                return;
            }
            #[cfg(test)]
            if self.manual_monitor {
                state = self.changed.wait(state).unwrap();
                continue;
            }
            if state.polling == 0 && state.ready.is_empty() {
                observations.clear();
                state = self.changed.wait(state).unwrap();
                continue;
            }
            self.scan(&mut state, &mut observations, Instant::now());
            // Notifications can trigger an earlier inspection; eligibility still
            // uses elapsed time for the *same* observed syscall generation.
            state = self
                .changed
                .wait_timeout(state, self.handoff_delay)
                .unwrap()
                .0;
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
                    *observation = Some((ticket, now));
                    continue;
                }
            };
            if now.saturating_duration_since(since) < self.handoff_delay
                || state.permits < self.parallelism
                || (state.ready.is_empty() && state.returning.is_empty())
            {
                continue;
            }

            // A returning caller already supplies a replacement OS thread. For
            // queued tasks, reserve a spare before changing permit ownership.
            if state.returning.is_empty() && state.workers.iter().all(|entry| entry.busy) {
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
                self.changed.notify_all();
            }
        }
    }
}

#[cfg(all(test, not(feature = "loom")))]
mod tests;
