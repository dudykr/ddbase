//! Bounded models of the production CallState and its surrounding locked
//! permit/park protocol. Task bodies and async-task's own waker state machine
//! are abstracted; these are not a model of the entire executor.

use std::collections::VecDeque;

use loom::{
    sync::{Arc, Condvar, Mutex},
    thread,
};

use super::CallState;

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
    loom::model(|| {
        // The runtime puts queue mutation, shutdown and the wait predicate
        // behind the same mutex. Notification can occur before or after park.
        // Queue, registered tasks, admission closed. Registration can precede
        // enqueue, so shutdown must not mistake an empty queue for completion.
        let gate = Arc::new((Mutex::new((VecDeque::new(), 0, false)), Condvar::new()));
        let worker_gate = gate.clone();
        let worker = thread::spawn(move || {
            let mut consumed = 0;
            let mut state = worker_gate.0.lock().unwrap();
            loop {
                if state.0.pop_front().is_some() {
                    consumed += 1;
                    state.1 -= 1;
                } else if state.2 && state.1 == 0 {
                    return consumed;
                } else {
                    state = worker_gate.1.wait(state).unwrap();
                }
            }
        });
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
                producer_gate.1.notify_all();
            }
            accepted
        });
        gate.0.lock().unwrap().2 = true;
        gate.1.notify_all();
        let accepted = producer.join().unwrap();
        assert_eq!(worker.join().unwrap(), usize::from(accepted));
    });
}
