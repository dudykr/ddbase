pub struct Interned {
    ptr: *const (),
}

impl Interned {
    pub fn new(ptr: *const ()) -> Self {
        Self { ptr }
    }
}
