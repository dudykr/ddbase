use super::{capacity::Capacity, Repr};

pub struct HeapStr {
    ptr: *const u8,
    len: Capacity,
}

static_assertions::assert_eq_size!(HeapStr, Repr);

impl HeapStr {
    pub unsafe fn new(text: &str) -> Self {
        let len = Capacity::new(text.len());
        let ptr = text.as_ptr();
        Self { ptr, len }
    }

    pub fn len(&self) -> usize {
        unsafe { self.len.as_usize() }
    }

    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(self.ptr, self.len())) }
    }
}
