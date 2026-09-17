use std::{
    any::Any,
    fmt,
    future::{pending, Future, Pending},
    panic::{catch_unwind, AssertUnwindSafe},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll, Waker},
};

use futures::{
    channel::oneshot,
    future::{AbortHandle, AbortRegistration, Abortable},
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
    future: Option<Pin<Box<F>>>,
    cancellation: Abortable<Pending<()>>,
    cancellation_waker: Option<Waker>,
    sender: Option<oneshot::Sender<Result<F::Output, JoinError>>>,
    shared: Arc<Shared>,
    id: TaskId,
}

impl<F: Future> TaskFuture<F> {
    pub(crate) fn new(
        future: F,
        registration: AbortRegistration,
        sender: oneshot::Sender<Result<F::Output, JoinError>>,
        shared: Arc<Shared>,
        id: TaskId,
    ) -> Self {
        Self {
            future: Some(Box::pin(future)),
            cancellation: Abortable::new(pending(), registration),
            cancellation_waker: None,
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
        if self.cancellation.is_aborted() {
            self.finish(Err(JoinError::cancelled()));
            return Poll::Ready(());
        }
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.future
                .as_mut()
                .expect("task polled after completion")
                .as_mut()
                .poll(cx)
        }));
        let result = match result {
            Ok(Poll::Pending) => {
                // The async-task waker normally stays the same across polls,
                // including worker migration. Keep its abort registration
                // instead of taking AtomicWaker's registration lock each time.
                // Only abort consumes that registration, and the aborted bit
                // remains set permanently afterwards.
                // Re-polling Abortable on a new waker preserves its existing
                // register/check handshake with a concurrent abort.
                let cancelled = if self
                    .cancellation_waker
                    .as_ref()
                    .is_some_and(|waker| waker.will_wake(cx.waker()))
                {
                    self.cancellation.is_aborted()
                } else {
                    self.cancellation_waker = Some(cx.waker().clone());
                    Pin::new(&mut self.cancellation).poll(cx).is_ready()
                };
                if !cancelled {
                    return Poll::Pending;
                }
                Err(JoinError::cancelled())
            }
            Ok(Poll::Ready(value)) => Ok(value),
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
