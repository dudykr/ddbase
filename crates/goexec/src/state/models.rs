//! Bounded models of the production CallState and its surrounding locked
//! permit/park protocol. Task bodies and async-task's own waker state machine
//! are abstracted; these are not a model of the entire executor.

use std::collections::VecDeque;

use loom::{
    sync::{
        atomic::{fence, AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
};

use super::CallState;

#[test]
fn published_work_or_a_notification_prevents_a_lost_park() {
    loom::model(|| {
        let queued = Arc::new(AtomicBool::new(false));
        let signal = Arc::new(AtomicBool::new(false));
        let gate = Arc::new((Mutex::new(()), Condvar::new()));
        let producer_queued = queued.clone();
        let producer_signal = signal.clone();
        let producer_gate = gate.clone();
        let producer = thread::spawn(move || {
            // Abstract release publication by enqueue or a batch steal into
            // its destination queue, including a temporarily hidden transfer.
            producer_queued.store(true, Ordering::Release);
            fence(Ordering::SeqCst);
            if producer_signal.load(Ordering::Acquire)
                && producer_signal.swap(false, Ordering::AcqRel)
            {
                let _guard = producer_gate.0.lock().unwrap();
                producer_gate.1.notify_one();
            }
        });
        let mut state = gate.0.lock().unwrap();
        loop {
            signal.store(true, Ordering::Release);
            fence(Ordering::SeqCst);
            if queued.load(Ordering::Acquire) {
                break;
            }
            state = gate.1.wait(state).unwrap();
        }
        drop(state);
        producer.join().unwrap();
    });
}

#[test]
fn closing_admission_cannot_miss_a_registered_task_in_a_shard() {
    loom::model(|| {
        let closed = Arc::new(AtomicBool::new(false));
        let registry = Arc::new(Mutex::new(false));
        let producer_closed = closed.clone();
        let producer_registry = registry.clone();
        let producer = thread::spawn(move || {
            let mut registered = producer_registry.lock().unwrap();
            if producer_closed.load(Ordering::Acquire) {
                return false;
            }
            *registered = true;
            true
        });
        closed.store(true, Ordering::Release);
        let seen = *registry.lock().unwrap();
        let accepted = producer.join().unwrap();
        assert!(!accepted || seen, "shutdown missed a registered task");
    });
}

#[test]
fn return_handoff_and_replacement_share_one_permit() {
    loom::model(|| {
        let call = Arc::new(CallState::new());
        let ticket = call.enter();
        let gate = Arc::new((Mutex::new(1usize), Condvar::new()));
        let monitor_call = call.clone();
        let monitor_gate = gate.clone();
        let monitor = thread::spawn(move || {
            let mut permits = monitor_gate.0.lock().unwrap();
            if monitor_call.detach(ticket) {
                assert_eq!(*permits, 1);
                *permits -= 1;
                monitor_gate.1.notify_all();
            }
        });
        let replacement_gate = gate.clone();
        let replacement = thread::spawn(move || {
            let mut permits = replacement_gate.0.lock().unwrap();
            while *permits == 1 {
                permits = replacement_gate.1.wait(permits).unwrap();
            }
            *permits += 1;
            drop(permits);
            thread::yield_now(); // Replacement user code owns the permit here.
            let mut permits = replacement_gate.0.lock().unwrap();
            assert_eq!(*permits, 1);
            *permits -= 1;
            replacement_gate.1.notify_all();
        });
        if !call.exit_fast(ticket) {
            let mut permits = gate.0.lock().unwrap();
            while *permits == 1 {
                permits = gate.1.wait(permits).unwrap();
            }
            *permits += 1;
            call.resume(ticket);
        }
        // Original poll can finish only after keeping or reacquiring its permit.
        let mut permits = gate.0.lock().unwrap();
        assert_eq!(*permits, 1);
        *permits -= 1;
        gate.1.notify_all();
        drop(permits);
        monitor.join().unwrap();
        replacement.join().unwrap();
        assert_eq!(*gate.0.lock().unwrap(), 0);
        assert!(call.syscall().is_none());
    });
}

#[test]
fn stale_observation_cannot_reclaim_a_later_call() {
    loom::model(|| {
        let call = Arc::new(CallState::new());
        let old = call.enter();
        let gate = Arc::new(Mutex::new(()));
        let monitor_call = call.clone();
        let monitor_gate = gate.clone();
        let monitor = thread::spawn(move || {
            let _guard = monitor_gate.lock().unwrap();
            monitor_call.detach(old);
        });
        if !call.exit_fast(old) {
            let _guard = gate.lock().unwrap();
            call.resume(old);
        }
        let new = call.enter();
        assert_ne!(old, new);
        monitor.join().unwrap();
        assert_eq!(call.syscall(), Some(new));
        assert!(call.exit_fast(new));
    });
}

#[test]
fn enqueue_and_shutdown_cannot_lose_a_parked_worker() {
    // Two workers plus registration/enqueue/close need a bounded schedule search.
    let mut model = loom::model::Builder::new();
    model.preemption_bound = Some(2);
    model.check(|| {
        // The runtime puts queue mutation, shutdown and the wait predicate
        // behind the same mutex. Notification can occur before or after park.
        // Queue, registered tasks, admission closed. Registration can precede
        // enqueue, so shutdown must not mistake an empty queue for completion.
        let gate = Arc::new((Mutex::new((VecDeque::new(), 0, false)), Condvar::new()));
        let workers = (0..2)
            .map(|_| {
                let worker_gate = gate.clone();
                thread::spawn(move || {
                    let mut consumed = 0;
                    let mut state = worker_gate.0.lock().unwrap();
                    loop {
                        if state.0.pop_front().is_some() {
                            consumed += 1;
                            state.1 -= 1;
                            if state.2 && state.1 == 0 {
                                // The last completion must also wake workers
                                // that reparked after the admission-close wake.
                                worker_gate.1.notify_all();
                            }
                        } else if state.2 && state.1 == 0 {
                            return consumed;
                        } else {
                            state = worker_gate.1.wait(state).unwrap();
                        }
                    }
                })
            })
            .collect::<Vec<_>>();
        let producer_gate = gate.clone();
        let producer = thread::spawn(move || {
            let mut state = producer_gate.0.lock().unwrap();
            let accepted = !state.2;
            if accepted {
                state.1 += 1;
            }
            drop(state);
            if accepted {
                thread::yield_now();
                producer_gate.0.lock().unwrap().0.push_back(());
                producer_gate.1.notify_one();
            }
            accepted
        });
        gate.0.lock().unwrap().2 = true;
        gate.1.notify_all();
        let accepted = producer.join().unwrap();
        let consumed: usize = workers.into_iter().map(|w| w.join().unwrap()).sum();
        assert_eq!(consumed, usize::from(accepted));
    });
}

#[test]
fn returning_fifo_wakes_the_head_not_an_arbitrary_waiter() {
    loom::model(|| {
        let state = Arc::new(Mutex::new((1usize, VecDeque::from([0, 1]), Vec::new())));
        let returned = Arc::new([Condvar::new(), Condvar::new()]);
        let workers = (0..2)
            .map(|id| {
                let gate = state.clone();
                let returned = returned.clone();
                thread::spawn(move || {
                    let mut state = gate.lock().unwrap();
                    while state.0 == 1 || state.1.front() != Some(&id) {
                        state = returned[id].wait(state).unwrap();
                    }
                    state.1.pop_front();
                    state.0 += 1;
                    state.2.push(id);
                    drop(state);
                    thread::yield_now();
                    let mut state = gate.lock().unwrap();
                    state.0 -= 1;
                    if let Some(&next) = state.1.front() {
                        returned[next].notify_one();
                    }
                })
            })
            .collect::<Vec<_>>();
        {
            let mut state = state.lock().unwrap();
            state.0 -= 1;
            returned[*state.1.front().unwrap()].notify_one();
        }
        for worker in workers {
            worker.join().unwrap();
        }
        let state = state.lock().unwrap();
        assert_eq!(state.0, 0);
        assert_eq!(state.2, [0, 1]);
    });
}
