//! Executor-independent timers backed by async-io's shared timer driver.
//!
//! Waiting never blocks a worker or creates a thread per timer. The driver is
//! process-global and may outlive a goexec runtime. Dropping a timer removes
//! its registration. Timeouts are cooperative: they cannot interrupt a poll,
//! including a synchronous call inside [`crate::blocking`].

pub use std::time::{Duration, Instant};
use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

/// A cancellable wait until a monotonic deadline.
#[derive(Debug)]
pub struct Sleep(async_io::Timer);

/// Wait for a duration. An unrepresentable deadline never fires.
pub fn sleep(duration: Duration) -> Sleep {
    Sleep(async_io::Timer::after(duration))
}

/// Wait until a monotonic deadline, immediately if it has already passed.
pub fn sleep_until(deadline: Instant) -> Sleep {
    Sleep(async_io::Timer::at(deadline))
}

impl Sleep {
    /// Change the deadline, preserving the registered task's waker.
    pub fn reset(&mut self, deadline: Instant) {
        self.0.set_at(deadline);
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        Pin::new(&mut self.0).poll(cx).map(|_| ())
    }
}

/// A future did not complete before its deadline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Elapsed;

impl fmt::Display for Elapsed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("deadline has elapsed")
    }
}
impl std::error::Error for Elapsed {}

/// A future raced against a timer. Dropping it drops the enclosed future.
#[must_use = "futures do nothing unless polled"]
pub struct Timeout<F> {
    future: Pin<Box<F>>,
    delay: Sleep,
}

/// Limit a future's duration. A ready future wins over a ready timer.
pub fn timeout<F: Future>(duration: Duration, future: F) -> Timeout<F> {
    Timeout {
        future: Box::pin(future),
        delay: sleep(duration),
    }
}

/// Limit a future to a monotonic deadline.
pub fn timeout_at<F: Future>(deadline: Instant, future: F) -> Timeout<F> {
    Timeout {
        future: Box::pin(future),
        delay: sleep_until(deadline),
    }
}

impl<F: Future> Future for Timeout<F> {
    type Output = Result<F::Output, Elapsed>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Poll::Ready(value) = self.future.as_mut().poll(cx) {
            return Poll::Ready(Ok(value));
        }
        Pin::new(&mut self.delay).poll(cx).map(|()| Err(Elapsed))
    }
}

/// How an interval schedules its next tick after missing a deadline by more
/// than five milliseconds (matching Tokio's tolerance for timer jitter).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MissedTickBehavior {
    /// Preserve every scheduled tick, including overdue ticks.
    #[default]
    Burst,
    /// Schedule the next tick one period after the late observation.
    Delay,
    /// Skip overdue ticks and preserve the original cadence.
    Skip,
}

/// A periodic timer. The first tick is immediately ready.
#[derive(Debug)]
pub struct Interval {
    delay: Sleep,
    deadline: Option<Instant>,
    period: Duration,
    behavior: MissedTickBehavior,
}

/// Create an interval whose first tick is immediate. Panics for a zero period.
pub fn interval(period: Duration) -> Interval {
    assert!(!period.is_zero(), "interval period must be nonzero");
    let deadline = Instant::now();
    Interval {
        delay: sleep_until(deadline),
        deadline: Some(deadline),
        period,
        behavior: MissedTickBehavior::Burst,
    }
}

impl Interval {
    /// Choose the scheduling policy for missed ticks.
    pub fn set_missed_tick_behavior(&mut self, behavior: MissedTickBehavior) {
        self.behavior = behavior;
    }

    /// Wait for the next tick and return its scheduled instant.
    /// Cancelling this wait does not consume a tick.
    pub async fn tick(&mut self) -> Instant {
        futures::future::poll_fn(|cx| self.poll_tick(cx)).await
    }

    /// Poll the next tick, registering this task's waker while pending.
    pub fn poll_tick(&mut self, cx: &mut Context<'_>) -> Poll<Instant> {
        let Some(deadline) = self.deadline else {
            return Poll::Pending;
        };
        if Pin::new(&mut self.delay).poll(cx).is_pending() {
            return Poll::Pending;
        }
        self.deadline = next_deadline(deadline, Instant::now(), self.period, self.behavior);
        match self.deadline {
            Some(next) => self.delay.reset(next),
            None => self.delay = Sleep(async_io::Timer::never()),
        }
        Poll::Ready(deadline)
    }
}

fn next_deadline(
    deadline: Instant,
    now: Instant,
    period: Duration,
    behavior: MissedTickBehavior,
) -> Option<Instant> {
    let late = now.saturating_duration_since(deadline);
    if late <= Duration::from_millis(5) {
        return deadline.checked_add(period);
    }
    match behavior {
        MissedTickBehavior::Burst => deadline.checked_add(period),
        MissedTickBehavior::Delay => now.checked_add(period),
        MissedTickBehavior::Skip => {
            let rem = late.as_nanos() % period.as_nanos();
            let rem = Duration::new((rem / 1_000_000_000) as u64, (rem % 1_000_000_000) as u32);
            now.checked_add(period - rem)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missed_tick_policies_use_scheduled_deadlines() {
        let start = Instant::now();
        let period = Duration::from_millis(10);
        let late = start + Duration::from_millis(35);
        assert_eq!(
            next_deadline(start, late, period, MissedTickBehavior::Burst),
            Some(start + period)
        );
        assert_eq!(
            next_deadline(start, late, period, MissedTickBehavior::Delay),
            Some(start + Duration::from_millis(45))
        );
        assert_eq!(
            next_deadline(start, late, period, MissedTickBehavior::Skip),
            Some(start + Duration::from_millis(40))
        );
        assert_eq!(
            next_deadline(
                start,
                start + Duration::from_millis(3),
                period,
                MissedTickBehavior::Delay
            ),
            Some(start + period)
        );
    }
}
