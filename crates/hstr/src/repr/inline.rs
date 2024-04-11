use super::{Repr, MAX_SIZE};

#[repr(transparent)]
pub struct InlineBuffer(pub [u8; MAX_SIZE]);
static_assertions::assert_eq_size!(InlineBuffer, Repr);

impl InlineBuffer {
    /// Safety: `text.len()` must be less than `MAX_SIZE`.
    pub unsafe fn new(text: &str) -> Self {
        let mut buffer = InlineBuffer([0; MAX_SIZE]);
        let len = text.len();
        let text = text.as_bytes();
        buffer.0[..len].copy_from_slice(text);
        buffer
    }
}
