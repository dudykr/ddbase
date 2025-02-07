//! **NOT A PUBLIC API.**
//!
//! Autoderef specialization for [`ShrinkToFit`].
//!
//!
//! Taken from next.js repository. https://github.com/vercel/next.js/blob/c9440f93fa5a35d6f489300a146e404936fbcbc9/turbopack/crates/turbo-tasks/src/macro_helpers.rs#L65

use std::ops::{Deref, DerefMut};

use crate::ShrinkToFit;

/// A wrapper type that uses the [autoderef specialization hack][autoderef] to
/// call [`ShrinkToFit::shrink_to_fit`] on types that implement [`ShrinkToFit`].
///
/// This uses a a no-op method [`ShrinkToFitFallbackNoop::shrink_to_fit`] on
/// types that do not implement [`ShrinkToFit`].
///
/// This is used by the derive macro for [`ShrinkToFit`], which is called by the
/// [turbo_tasks::value][macro@crate::value] macro.
///
/// [autoderef]: http://lukaskalbertodt.github.io/2019/12/05/generalized-autoref-based-specialization.html
pub struct ShrinkToFitDerefSpecialization<'a, T> {
    inner: ShrinkToFitFallbackNoop<'a, T>,
}

impl<'a, T> ShrinkToFitDerefSpecialization<'a, T> {
    pub fn new(real: &'a mut T) -> Self {
        Self {
            inner: ShrinkToFitFallbackNoop { real },
        }
    }
}

impl<T> ShrinkToFitDerefSpecialization<'_, T>
where
    T: ShrinkToFit,
{
    pub fn shrink_to_fit(&mut self) {
        // call the real `ShrinkToFit::shrink_to_fit` method
        self.inner.real.shrink_to_fit()
    }
}

impl<'a, T> Deref for ShrinkToFitDerefSpecialization<'a, T> {
    type Target = ShrinkToFitFallbackNoop<'a, T>;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T> DerefMut for ShrinkToFitDerefSpecialization<'_, T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

// Implements `ShrinkToFit` using a no-op `ShrinkToFit::shrink_to_fit` method.
pub struct ShrinkToFitFallbackNoop<'a, T> {
    real: &'a mut T,
}

impl<T> ShrinkToFitFallbackNoop<'_, T> {
    /// A no-op function called as part of [`ShrinkToFitDerefSpecialization`]
    /// when `T` does not implement [`ShrinkToFit`].
    pub fn shrink_to_fit(&mut self) {}
}
