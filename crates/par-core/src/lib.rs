//! # par-core
//!
//! A wrapper for various parallelization library for Rust.
//! This crate currently supports
//!
//! - [`chili`](https://github.com/dragostis/chili)
//! - [`rayon`](https://github.com/rayon-rs/rayon)
//! - Disable parallelization.
//!
//! # Usage
//!
//! If you are developing a library, you should not force the parallelization
//! library, and let the users choose the parallelization library.
//!
//! ## Final application
//!
//! If you are developing a final application, you can use cargo feature to
//! select the parallelization library.
//!
//! ### `chili`
//!
//! ```toml
//! [dependencies]
//! par-core = { version = "1.0.1", features = ["chili"] }
//! ```
//!
//! ### `rayon`
//!
//! ```toml
//! [dependencies]
//! par-core = { version = "1.0.1", features = ["rayon"] }
//! ```
//!
//! ### Disable parallelization
//!
//! ```toml
//! [dependencies]
//! par-core = { version = "1.0.1", default-features = false }
//! ```
//!
//! ## Library developers
//!
//! If you are developing a library, you can simply depend on `par-core` without
//! any features. **Note**: To prevent a small mistake of end-user making the
//! appplication slower, `par-core` emits a error message using a default
//! feature. So if you are a library developer, you should specify
//! `default-features = false`.
//!
//! ```toml
//! [dependencies]
//! par-core = { version = "1.0.1", default-features = false }
//! ```

#[cfg(all(not(feature = "chili"), not(feature = "rayon"), feature = "parallel"))]
compile_error!("You must enable `chili` or `rayon` feature if you want to use `parallel` feature");

#[cfg(all(feature = "chili", feature = "rayon"))]
compile_error!("You must enable `chili` or `rayon` feature, not both");

#[cfg(feature = "chili")]
mod par_chili {
    use std::{cell::RefCell, mem::transmute};

    thread_local! {
        static SCOPE: RefCell<Option<MaybeScope<'static>>> = Default::default();
    }

    #[derive(Default)]
    struct MaybeScope<'a>(ScopeLike<'a>);

    struct Scope<'a>(&'a mut chili::Scope<'a>);

    enum ScopeLike<'a> {
        Scope(Scope<'a>),
        Global(Option<chili::Scope<'a>>),
    }

    impl Default for ScopeLike<'_> {
        fn default() -> Self {
            ScopeLike::Global(None)
        }
    }

    impl<'a> MaybeScope<'a> {
        #[allow(clippy::redundant_closure)]
        fn with<F, R>(&mut self, f: F) -> R
        where
            F: FnOnce(Scope<'a>) -> R,
        {
            let scope: &mut chili::Scope = match &mut self.0 {
                ScopeLike::Scope(scope) => unsafe {
                    // Safety: chili Scope will be alive until the end of the function, because it's
                    // contract of 'a lifetime in the type.

                    transmute::<&mut chili::Scope, &mut chili::Scope>(&mut scope.0)
                },
                ScopeLike::Global(global_scope) => {
                    // Initialize global scope lazily, and only once.
                    let scope = global_scope.get_or_insert_with(|| chili::Scope::global());

                    unsafe {
                        // Safety: Global scope is not dropped until the end of the program, and no
                        // one can access this **instance** of the global
                        // scope in the same time.
                        transmute::<&mut chili::Scope, &mut chili::Scope>(scope)
                    }
                }
            };

            let scope = Scope(scope);

            f(scope)
        }
    }

    #[inline]
    fn join_maybe_scoped<'a, A, B, RA, RB>(
        scope: &mut MaybeScope<'a>,
        oper_a: A,
        oper_b: B,
    ) -> (RA, RB)
    where
        A: Send + FnOnce(Scope<'a>) -> RA,
        B: Send + FnOnce(Scope<'a>) -> RB,
        RA: Send,
        RB: Send,
    {
        scope.with(|scope| join_scoped(scope, oper_a, oper_b))
    }

    #[inline]
    fn join_scoped<'a, A, B, RA, RB>(scope: Scope<'a>, oper_a: A, oper_b: B) -> (RA, RB)
    where
        A: Send + FnOnce(Scope<'a>) -> RA,
        B: Send + FnOnce(Scope<'a>) -> RB,
        RA: Send,
        RB: Send,
    {
        let (ra, rb) = scope.0.join(
            |scope| {
                let scope = Scope(unsafe {
                    // Safety: This can be dangerous if the user do transmute on the scope, but it's
                    // not our fault if the user uses transmute.
                    transmute::<&mut chili::Scope, &mut chili::Scope>(scope)
                });

                oper_a(scope)
            },
            |scope| {
                let scope = Scope(unsafe {
                    // Safety: This can be dangerous if the user do transmute on the scope, but it's
                    // not our fault if the user uses transmute.
                    transmute::<&mut chili::Scope, &mut chili::Scope>(scope)
                });

                oper_b(scope)
            },
        );

        (ra, rb)
    }

    #[inline]
    pub fn join<A, B, RA, RB>(oper_a: A, oper_b: B) -> (RA, RB)
    where
        A: Send + FnOnce() -> RA,
        B: Send + FnOnce() -> RB,
        RA: Send,
        RB: Send,
    {
        struct RemoveScopeGuard;

        impl Drop for RemoveScopeGuard {
            fn drop(&mut self) {
                SCOPE.set(None);
            }
        }

        let mut scope = SCOPE.take().unwrap_or_default();

        let (ra, rb) = join_maybe_scoped(
            &mut scope,
            |scope| {
                let scope = unsafe {
                    // Safety: inner scope cannot outlive the outer scope
                    transmute::<Scope, Scope>(scope)
                };
                let _guard = RemoveScopeGuard;
                SCOPE.set(Some(MaybeScope(ScopeLike::Scope(scope))));

                oper_a()
            },
            |scope| {
                let scope = unsafe {
                    // Safety: inner scope cannot outlive the outer scope
                    transmute::<Scope, Scope>(scope)
                };
                let _guard = RemoveScopeGuard;
                SCOPE.set(Some(MaybeScope(ScopeLike::Scope(scope))));

                oper_b()
            },
        );

        // In case of panic, we does not restore the scope so it will be None.
        SCOPE.set(Some(scope));

        (ra, rb)
    }
}

pub fn join<A, B, RA, RB>(oper_a: A, oper_b: B) -> (RA, RB)
where
    A: Send + FnOnce() -> RA,
    B: Send + FnOnce() -> RB,
    RA: Send,
    RB: Send,
{
    #[cfg(feature = "chili")]
    let (ra, rb) = par_chili::join(oper_a, oper_b);

    #[cfg(feature = "rayon")]
    let (ra, rb) = rayon::join(oper_a, oper_b);

    #[cfg(not(feature = "parallel"))]
    let (ra, rb) = (oper_a(), oper_b());

    (ra, rb)
}
