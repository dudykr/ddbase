use super::{capacity::Capacity, Repr};

#[repr(C)]
pub(super) struct StaticStr {
    ptr: *const u8,
    len: Capacity,
}

static_assertions::assert_eq_size!(Repr, StaticStr);

impl StaticStr {
    pub unsafe fn new(text: &'static str) -> Self {
        let len = Capacity::new(text.len());

        Self {
            ptr: text.as_ptr(),
            len,
        }
    }

    pub fn len(&self) -> usize {
        unsafe { self.len.as_usize() }
    }

    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr, self.len())) }
    }
}
