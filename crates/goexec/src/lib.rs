//! A cooperative executor with deferred handoff of explicitly marked blocking
//! calls.
//!
//! Tasks remain stackless Rust futures. A blocking call runs on its caller's OS
//! thread. If it takes long enough and other tasks need to run, a monitor lets
//! another worker take over its execution permit.
//!
//! ```
//! let runtime = goexec::Runtime::builder().parallelism(2).build()?;
//! let answer = runtime.block_on(async {
//!     goexec::spawn(async { goexec::blocking(|| 42) }).await.unwrap()
//! });
//! assert_eq!(answer, 42);
//! assert!(runtime.shutdown_timeout(std::time::Duration::from_secs(1)));
//! # Ok::<(), std::io::Error>(())
//! ```
//!
//! Only [`blocking`] and [`fs`] calls participate in handoff. CPU loops must
//! cooperate using [`yield_now`]. A blocking call also blocks sibling futures
//! in the **same task** (`join!`, `select!`, or a timeout). Spawn independent
//! tasks when they must make progress concurrently.
//!
//! This crate supplies neither a network/timer driver nor Tokio API
//! compatibility. The `loom` feature is only for the internal model tests.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod fs;
mod runtime;
mod state;
mod task;

pub use runtime::{blocking, spawn, yield_now, Builder, Handle, Metrics, Runtime};
pub use task::{JoinError, JoinHandle};
