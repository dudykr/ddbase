use std::mem::transmute;

use super::{nonmax::NonMaxU8, Repr, MAX_SIZE};

pub struct InlineBuffer(pub [u8; MAX_SIZE - 1], NonMaxU8);
static_assertions::assert_eq_size!(InlineBuffer, Repr);

impl InlineBuffer {
    /// Safety: `text.len()` must be less than `MAX_SIZE`.
    pub unsafe fn new(text: &str) -> Self {
        let mut buffer = InlineBuffer([0; MAX_SIZE - 1], unsafe { transmute(text.len() as u8) });
        let len = text.len();
        let text = text.as_bytes();
        buffer.0[..len].copy_from_slice(text);
        buffer
    }

    pub fn len(&self) -> usize {
        unsafe { transmute::<_, u8>(self.1) as usize }
    }

    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.0[..self.len()]) }
    }
}
