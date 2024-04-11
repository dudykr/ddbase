use std::mem::size_of;

use debug_unreachable::debug_unreachable;

use self::{inline::InlineBuffer, nonmax::NonMaxUsize, static_ref::StaticStr};

mod heap;
mod inline;
mod interned;
mod nonmax;
mod static_ref;

const MAX_SIZE: usize = size_of::<Repr>();

#[repr(C)]
pub struct Repr(
    // We have a pointer in the repesentation to properly carry provenance
    *const u8,
    NonMaxUsize,
);

unsafe impl Send for Repr {}
unsafe impl Sync for Repr {}

const KIND_INLINED: u8 = 0b00;
const KIND_INTERNED: u8 = 0b01;
const KIND_HEAP: u8 = 0b10;
const KIND_STATIC: u8 = 0b11;
const KIND_MASK: u8 = 0b11;

impl Repr {
    #[inline]
    pub fn new_static(text: &'static str) -> Self {
        let repr = unsafe { StaticStr::new(text) };
        let repr = unsafe { std::mem::transmute::<StaticStr, Repr>(repr) };

        if cfg!(feature = "debug") {
            assert_eq!(repr.as_str(), text);
            assert_eq!(repr.kind(), KIND_STATIC);
        }

        repr
    }

    #[inline]
    pub fn new_dynamic(text: &str) -> Self {
        let len = text.len();

        if len == 0 {
            return Self::new_static("");
        }

        if len < MAX_SIZE {
            let repr = unsafe { InlineBuffer::new(text) };
            let repr = unsafe { std::mem::transmute::<InlineBuffer, Repr>(repr) };

            if cfg!(feature = "debug") {
                assert_eq!(repr.as_str(), text);
                assert_eq!(repr.kind(), KIND_INLINED);
            }

            repr
        } else {
            let repr = unsafe { heap::HeapStr::new(text) };
            let repr = unsafe { std::mem::transmute::<heap::HeapStr, Repr>(repr) };

            if cfg!(feature = "debug") {
                assert_eq!(repr.as_str(), text);
                assert_eq!(repr.kind(), KIND_HEAP);
            }

            repr
        }
    }

    #[inline]
    pub fn new_interned(text: &str) -> Self {}

    fn len(&self) -> usize {
        match self.kind() {
            KIND_INLINED => {}
            KIND_HEAP => {}
            KIND_STATIC => {
                let repr = unsafe { std::mem::transmute::<Repr, StaticStr>(*self) };
                repr.len()
            }
            KIND_INTERNED => {}
            _ => unsafe { debug_unreachable!("Invalid kind in Repr::len()") },
        }
    }

    fn as_str(&self) -> &str {}

    #[inline]
    fn kind(&self) -> u8 {
        self.last_byte() & KIND_MASK
    }

    fn last_byte(&self) -> u8 {
        self.1.last_byte()
    }
}

static_assertions::assert_eq_size!(Repr, Option<Repr>, [usize; 2]);
