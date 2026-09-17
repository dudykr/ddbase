use std::{cell::Cell, task::Poll};

use futures::future::poll_fn;

thread_local! {
    static BUDGET: Cell<Option<u8>> = const { Cell::new(None) };
}

pub(crate) struct Guard(Option<u8>);

pub(crate) fn enter() -> Guard {
    Guard(BUDGET.with(|budget| budget.replace(Some(128))))
}

impl Drop for Guard {
    fn drop(&mut self) {
        BUDGET.with(|budget| budget.set(self.0));
    }
}

/// Consume one of the task's 128 cooperative checkpoints per poll.
///
/// Once the budget is exhausted, wake this task and yield until its next poll.
/// CPU loops should call this periodically. Outside a goexec task this is a
/// no-op. The budget is shared by sibling futures within a task, restored on
/// unwind, and reset whenever the executor polls the task again.
pub async fn consume_budget() {
    poll_fn(|cx| {
        BUDGET.with(|budget| match budget.get() {
            Some(0) => {
                cx.waker().wake_by_ref();
                Poll::Pending
            }
            Some(remaining) => {
                budget.set(Some(remaining - 1));
                Poll::Ready(())
            }
            None => Poll::Ready(()),
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use std::{future::Future, pin::pin, task::Context};

    use super::*;

    #[test]
    fn budget_is_shared_reset_and_restored_on_unwind() {
        let waker = futures::task::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = enter();
            for _ in 0..128 {
                assert!(pin!(consume_budget()).poll(&mut cx).is_ready());
            }
            assert!(pin!(consume_budget()).poll(&mut cx).is_pending());
            {
                let _nested = enter();
                assert!(pin!(consume_budget()).poll(&mut cx).is_ready());
            }
            assert!(pin!(consume_budget()).poll(&mut cx).is_pending());
            panic!("restore budget");
        }));
        assert!(result.is_err());
        assert!(BUDGET.with(Cell::get).is_none());
        assert!(pin!(consume_budget()).poll(&mut cx).is_ready());
    }
}
