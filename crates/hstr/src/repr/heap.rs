pub struct HeapStr {
    ptr: *const u8,
}

impl HeapStr {
    pub unsafe fn new(text: &str) -> Self {}
}
