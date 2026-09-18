use std::{
    any::Any,
    fmt,
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use async_task::{FallibleTask, Task};
use futures::future::{AbortHandle, Abortable};

use crate::runtime::{Shared, TaskId};

/// A task was cancelled or panicked.
pub struct JoinError {
    panic: Option<Box<dyn Any + Send + 'static>>,
}

impl JoinError {
    pub(crate) fn cancelled() -> Self {
        Self { panic: None }
    }

    fn panicked(panic: Box<dyn Any + Send + 'static>) -> Self {
        Self { panic: Some(panic) }
    }

    /// Whether the task was cancelled, including runtime shutdown.
    pub fn is_cancelled(&self) -> bool {
        self.panic.is_none()
    }

    /// Whether polling or dropping the task's future panicked.
    pub fn is_panic(&self) -> bool {
        self.panic.is_some()
    }

    /// Recover the panic payload. Panics if this is a cancellation error.
    pub fn into_panic(self) -> Box<dyn Any + Send + 'static> {
        self.panic.expect("task did not panic")
    }
}

impl fmt::Debug for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinError")
            .field("is_panic", &self.is_panic())
            .finish()
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.is_panic() {
            "task panicked"
        } else {
            "task was cancelled"
        })
    }
}

impl std::error::Error for JoinError {}

/// A task result. Dropping this handle detaches the task rather than cancelling
/// it. Awaiting the handle does not require a goexec runtime.
#[must_use = "await the handle to observe the task's result"]
pub struct JoinHandle<T> {
    task: Option<FallibleTask<TaskOutput<T>>>,
    abort: AbortHandle,
}

impl<T> JoinHandle<T> {
    pub(crate) fn new(task: Task<TaskOutput<T>>, abort: AbortHandle) -> Self {
        Self {
            task: Some(task.fallible()),
            abort,
        }
    }

    pub(crate) fn cancelled(abort: AbortHandle) -> Self {
        Self { task: None, abort }
    }

    /// Obtain an independently owned cancellation handle. Dropping it does not
    /// cancel the task; call `abort` explicitly.
    pub fn abort_handle(&self) -> AbortHandle {
        self.abort.clone()
    }

    /// Request cancellation at a poll boundary. A running syscall or CPU loop
    /// is not interrupted; completion may win a race with cancellation.
    pub fn abort(&self) {
        self.abort.abort();
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let Some(task) = self.task.as_mut() else {
            return Poll::Ready(Err(JoinError::cancelled()));
        };
        Pin::new(task).poll(cx).map(|output| {
            output.map_or_else(
                || Err(JoinError::cancelled()),
                |mut output| output.0.take().expect("missing task output"),
            )
        })
    }
}

impl<T> Drop for JoinHandle<T> {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.detach();
        }
    }
}

// async-task must never see a panic from an unobserved user result's
// destructor. The joiner takes the value out; detached/completed-but-unjoined
// values are destroyed here, whether on a worker or on the thread dropping the
// handle.
pub(crate) struct TaskOutput<T>(Option<Result<T, JoinError>>);

impl<T> Drop for TaskOutput<T> {
    fn drop(&mut self) {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(self.0.take())));
    }
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("JoinHandle").finish_non_exhaustive()
    }
}

// Keeping the user future in a pinned box lets us catch both poll and
// destructor panics without unsafe projection or allowing a destructor to
// escape into async-task's abort-on-destructor-panic boundary.
pub(crate) struct TaskFuture<F: Future> {
    future: Option<Pin<Box<Abortable<F>>>>,
    shared: Arc<Shared>,
    id: TaskId,
}

impl<F: Future> TaskFuture<F> {
    pub(crate) fn new(future: Abortable<F>, shared: Arc<Shared>, id: TaskId) -> Self {
        Self {
            future: Some(Box::pin(future)),
            shared,
            id,
        }
    }

    fn finish(&mut self, mut result: Result<F::Output, JoinError>) -> TaskOutput<F::Output> {
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(self.future.take()))) {
            let previous = std::mem::replace(&mut result, Err(JoinError::panicked(panic)));
            // Both a completed output and its future can have panicking
            // destructors. Keep either unwind out of async-task's drop glue.
            let _ = catch_unwind(AssertUnwindSafe(|| drop(previous)));
        }
        TaskOutput(Some(result))
    }
}

impl<F: Future> Unpin for TaskFuture<F> {}

impl<F: Future> Future for TaskFuture<F> {
    type Output = TaskOutput<F::Output>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.future
                .as_mut()
                .expect("task polled after completion")
                .as_mut()
                .poll(cx)
        }));
        let result = match result {
            Ok(Poll::Pending) => return Poll::Pending,
            Ok(Poll::Ready(Ok(value))) => Ok(value),
            Ok(Poll::Ready(Err(_))) => Err(JoinError::cancelled()),
            Err(panic) => Err(JoinError::panicked(panic)),
        };
        Poll::Ready(self.finish(result))
    }
}

impl<F: Future> Drop for TaskFuture<F> {
    fn drop(&mut self) {
        if self.future.is_some() {
            drop(self.finish(Err(JoinError::cancelled())));
        }
        self.shared.task_finished(self.id);
    }
}
