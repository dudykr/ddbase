//! The atomic half of permit ownership. The scheduler mutex protects the permit
//! count and serializes a detached caller's return with the monitor's handoff.

#[cfg(not(all(test, feature = "loom")))]
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(all(test, feature = "loom"))]
use loom::sync::atomic::{AtomicU64, Ordering};

const PHASE: u64 = 3;
const RUNNING: u64 = 0;
const SYSCALL: u64 = 1;
const DETACHED: u64 = 2;

pub(crate) struct CallState(AtomicU64);

impl CallState {
    pub(crate) fn new() -> Self {
        Self(AtomicU64::new(RUNNING))
    }

    // Only the worker calls enter, and only at the outermost blocking boundary.
    pub(crate) fn enter(&self) -> u64 {
        let old = self.0.load(Ordering::Relaxed);
        debug_assert_eq!(old & PHASE, RUNNING);
        let ticket = old.wrapping_add(4) | SYSCALL;
        self.0.store(ticket, Ordering::Release);
        ticket
    }

    pub(crate) fn syscall(&self) -> Option<u64> {
        let word = self.0.load(Ordering::Acquire);
        (word & PHASE == SYSCALL).then_some(word)
    }

    // Racing return and detach cannot both succeed. No scheduler lock is needed
    // when the caller wins. The generation rejects a stale monitor observation.
    pub(crate) fn exit_fast(&self, ticket: u64) -> bool {
        self.0
            .compare_exchange(ticket, ticket & !PHASE, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    // Caller must hold the scheduler mutex, reserve a replacement, and decrement
    // the permit count before unlocking. A losing exit_fast waits for this lock.
    pub(crate) fn detach(&self, ticket: u64) -> bool {
        self.0
            .compare_exchange(
                ticket,
                (ticket & !PHASE) | DETACHED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    // Called under the scheduler mutex after reacquiring a permit.
    pub(crate) fn resume(&self, ticket: u64) {
        debug_assert_eq!(self.0.load(Ordering::Acquire), (ticket & !PHASE) | DETACHED);
        self.0.store(ticket & !PHASE, Ordering::Release);
    }
}

#[cfg(all(test, feature = "loom"))]
mod models;
