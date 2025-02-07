pub(crate) trait MayShrinkToFit {
    fn may_shrink_to_fit(&mut self);
}

#[cfg(feature = "nightly")]
impl<T> MayShrinkToFit for T {
    default fn may_shrink_to_fit(&mut self) {}
}

#[cfg(feature = "nightly")]
impl<T: ShrinkToFit> MayShrinkToFit for T {
    fn may_shrink_to_fit(&mut self) {
        self.shrink_to_fit();
    }
}

/// Noop for non-nightly.
#[cfg(not(feature = "nightly"))]
impl<T> MayShrinkToFit for T {
    #[inline(always)]
    fn may_shrink_to_fit(&mut self) {}
}

pub(crate) fn may_shrink_to_fit<T: MayShrinkToFit>(value: &mut T) {
    value.may_shrink_to_fit();
}
