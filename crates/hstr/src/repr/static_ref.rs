use std::ptr;

use super::{capacity::Capacity, Repr};

#[repr(C)]
pub(super) struct StaticStr {
    ptr: ptr::NonNull<u8>,
    len: Capacity,
}

static_assertions::assert_eq_size!(Repr, StaticStr);

impl StaticStr {
    pub unsafe fn new(text: &'static str) -> Self {
        let len = Capacity::new(text.len());

        Self {
            ptr: ptr::NonNull::new_unchecked(text as *const str as *mut u8),
            len,
        }
    }

    pub fn len(&self) -> usize {
        unsafe { self.len.as_usize() }
    }

    pub fn as_str(&self) -> &str {
        unsafe {
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr.as_ptr(), self.len()))
        }
    }
}
