use super::{nonmax::NonMaxUsize, Repr, KIND_STATIC};

#[repr(C)]
pub(super) struct StaticStr {
    ptr: *const u8,
    /// We use the last two bits to store the kind of the string.
    len: NonMaxUsize,
}

static_assertions::assert_eq_size!(Repr, StaticStr);

const MAX_LEN: usize = (usize::MAX >> 2) - 1;

impl StaticStr {
    // Safety: `text.len()` must be less than `usize::MAX >> 2 - 1`.
    pub unsafe fn new(text: &'static str) -> Self {
        // Shift length to the right by 2 bits and store the kind in the last two
        // bits.

        debug_assert!(text.len() < MAX_LEN);
        let len = NonMaxUsize::new(text.len() << 2 | (KIND_STATIC as usize));

        Self {
            ptr: text.as_ptr(),
            len,
        }
    }

    pub fn len(&self) -> usize {
        self.len.as_usize() >> 2
    }

    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr, self.len())) }
    }
}
