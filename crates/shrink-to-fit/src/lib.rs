/// Recursively calls `shrink_to_fit` on all elements of the container.
pub trait ShrinkToFit {
    fn shrink_to_fit(&mut self);
}

macro_rules! impl_noop {
    ($($t:ty),*) => {
        $(
            impl ShrinkToFit for $t {
                #[inline(always)]
                fn shrink_to_fit(&mut self) {}
            }
        )*
    };
}

impl_noop!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);
impl_noop!(bool, char);

impl<T: ShrinkToFit> ShrinkToFit for Vec<T> {
    #[inline]
    fn shrink_to_fit(&mut self) {
        for value in self.iter_mut() {
            value.shrink_to_fit();
        }
        self.shrink_to_fit();
    }
}

impl ShrinkToFit for String {
    #[inline]
    fn shrink_to_fit(&mut self) {
        self.shrink_to_fit();
    }
}

impl<T: ShrinkToFit> ShrinkToFit for Box<T> {
    #[inline]
    fn shrink_to_fit(&mut self) {
        self.as_mut().shrink_to_fit();
    }
}

impl<T: ShrinkToFit> ShrinkToFit for Option<T> {
    #[inline]
    fn shrink_to_fit(&mut self) {
        if let Some(value) = self {
            value.shrink_to_fit();
        }
    }
}
