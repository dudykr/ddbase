pub trait ShrinkToFit {
    fn shrink_to_fit(&mut self);
}

impl<T: ShrinkToFit> ShrinkToFit for Vec<T> {
    fn shrink_to_fit(&mut self) {
        for value in self.iter_mut() {
            value.shrink_to_fit();
        }
        self.shrink_to_fit();
    }
}

impl ShrinkToFit for String {
    fn shrink_to_fit(&mut self) {
        self.shrink_to_fit();
    }
}

macro_rules! impl_noop {
    ($($t:ty),*) => {
        $(
            impl ShrinkToFit for $t {
                fn shrink_to_fit(&mut self) {}
            }
        )*
    };
}

impl_noop!(u8, u16, u32, u64, u128, i8, i16, i32, i64, i128, f32, f64);
impl_noop!(bool, char);
