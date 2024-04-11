use super::{nonmax::NonMaxUsize, Repr, KIND_STATIC};

pub(super) struct StaticStr {
    ptr: *const u8,
    /// We use the last two bits to store the kind of the string.
    len: NonMaxUsize,
}

static_assertions::assert_eq_size!(Repr, StaticStr);

impl StaticStr {
    pub fn new(text: &'static str) -> Self {
        // Shift length to the right by 2 bits and store the kind in the last two
        // bits.

        let len = NonMaxUsize::new(text.len() << 2 | (KIND_STATIC as usize));

        Self {
            ptr: text.as_ptr(),
            len,
        }
    }
}
