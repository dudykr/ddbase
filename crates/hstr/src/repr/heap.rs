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
}
