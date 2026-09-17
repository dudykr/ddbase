use std::{
    any::Any,
    fmt,
    future::Future,
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{
    channel::oneshot,
    future::{AbortHandle, Abortable},
};

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
    receiver: oneshot::Receiver<Result<T, JoinError>>,
    abort: AbortHandle,
}

impl<T> JoinHandle<T> {
    pub(crate) fn new(
        receiver: oneshot::Receiver<Result<T, JoinError>>,
        abort: AbortHandle,
    ) -> Self {
        Self { receiver, abort }
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
        Pin::new(&mut self.receiver)
            .poll(cx)
            .map(|result| result.unwrap_or_else(|_| Err(JoinError::cancelled())))
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
    sender: Option<oneshot::Sender<Result<F::Output, JoinError>>>,
    shared: Arc<Shared>,
    id: TaskId,
}

impl<F: Future> TaskFuture<F> {
    pub(crate) fn new(
        future: Abortable<F>,
        sender: oneshot::Sender<Result<F::Output, JoinError>>,
        shared: Arc<Shared>,
        id: TaskId,
    ) -> Self {
        Self {
            future: Some(Box::pin(future)),
            sender: Some(sender),
            shared,
            id,
        }
    }

    fn finish(&mut self, mut result: Result<F::Output, JoinError>) {
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| drop(self.future.take()))) {
            let previous = std::mem::replace(&mut result, Err(JoinError::panicked(panic)));
            // Both a completed output and its future can have panicking
            // destructors. Keep either unwind out of async-task's drop glue.
            let _ = catch_unwind(AssertUnwindSafe(|| drop(previous)));
        }
        if let Some(sender) = self.sender.take() {
            // A detached result may have a user-defined destructor. Keep its
            // panic out of async-task's own destructor.
            let _ = catch_unwind(AssertUnwindSafe(|| drop(sender.send(result))));
        }
    }
}

impl<F: Future> Unpin for TaskFuture<F> {}

impl<F: Future> Future for TaskFuture<F> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
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
        self.finish(result);
        Poll::Ready(())
    }
}

impl<F: Future> Drop for TaskFuture<F> {
    fn drop(&mut self) {
        if self.future.is_some() {
            self.finish(Err(JoinError::cancelled()));
        }
        self.shared.task_finished(self.id);
    }
}
