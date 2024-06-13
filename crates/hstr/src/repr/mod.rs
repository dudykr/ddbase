use std::mem::{size_of, transmute};

use debug_unreachable::debug_unreachable;

use self::{
    heap::HeapStr, inline::InlineBuffer, interned::Interned, nonmax::NonMaxUsize,
    static_ref::StaticStr,
};

mod capacity;
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

/// Used as a discriminant to identify different variants
const HEAP_MASK: u8 = 0b11111110;

impl Repr {
    #[inline]
    pub fn new_static(text: &'static str) -> Self {
        let repr = unsafe { StaticStr::new(text) };

        debug_assert_eq!(repr.len(), text.len());

        let repr = unsafe { std::mem::transmute::<StaticStr, Repr>(repr) };

        debug_assert_eq!(repr.kind(), KIND_STATIC);
        debug_assert_eq!(repr.len(), text.len());

        if cfg!(feature = "debug") {
            assert_eq!(repr.as_str(), text);
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

            debug_assert_eq!(repr.len(), text.len());

            let repr = unsafe { std::mem::transmute::<InlineBuffer, Repr>(repr) };

            debug_assert_eq!(repr.kind(), KIND_INLINED);
            debug_assert_eq!(repr.len(), text.len());

            if cfg!(feature = "debug") {
                assert_eq!(repr.as_str(), text);
            }

            repr
        } else {
            let repr = unsafe { HeapStr::new(text) };

            debug_assert_eq!(repr.len(), text.len());

            let repr = unsafe { std::mem::transmute::<HeapStr, Repr>(repr) };

            debug_assert_eq!(repr.kind(), KIND_HEAP);
            debug_assert_eq!(repr.len(), text.len());

            if cfg!(feature = "debug") {
                assert_eq!(repr.as_str(), text);
            }

            repr
        }
    }

    // #[inline]
    // pub fn new_interned(text: &str) -> Self {}

    fn len(&self) -> usize {
        match self.kind() {
            KIND_INLINED => {
                let repr = unsafe { std::mem::transmute::<&Repr, &InlineBuffer>(self) };
                repr.len()
            }
            KIND_HEAP => {
                let repr = unsafe { std::mem::transmute::<&Repr, &HeapStr>(self) };
                repr.len()
            }
            KIND_STATIC => {
                let repr = unsafe { std::mem::transmute::<&Repr, &StaticStr>(self) };
                repr.len()
            }
            KIND_INTERNED => {
                todo!("Repr::len() for interned strings")
            }
            _ => unsafe { debug_unreachable!("Invalid kind in Repr::len()") },
        }
    }

    fn as_str(&self) -> &str {
        match self.kind() {
            KIND_INLINED => {
                let repr = unsafe { std::mem::transmute::<&Repr, &InlineBuffer>(self) };
                repr.as_str()
            }
            KIND_HEAP => {
                let repr = unsafe { std::mem::transmute::<&Repr, &HeapStr>(self) };
                repr.as_str()
            }
            KIND_STATIC => {
                let repr = unsafe { std::mem::transmute::<&Repr, &StaticStr>(self) };
                repr.as_str()
            }
            KIND_INTERNED => {
                todo!("Repr::as_str() for interned strings")
            }
            _ => unsafe { debug_unreachable!("Invalid kind in Repr::as_str()") },
        }
    }

    #[inline]
    fn kind(&self) -> u8 {
        self.last_byte() & KIND_MASK
    }

    fn last_byte(&self) -> u8 {
        self.1.last_byte()
    }
}

static_assertions::assert_eq_size!(Repr, Option<Repr>, [usize; 2]);

impl Drop for Repr {
    #[inline]
    fn drop(&mut self) {
        // By "outlining" the actual Drop code and only calling it if we're a heap
        // variant, it allows dropping an inline variant to be as cheap as
        // possible.
        match self.kind() {
            KIND_HEAP | KIND_INLINED => outlined_drop(self),
            _ => {}
        }

        #[cold]
        fn outlined_drop(this: &mut Repr) {
            match this.kind() {
                KIND_HEAP => {
                    let repr = unsafe {
                        // SAFETY: We just checked the discriminant to make sure we're heap
                        // allocated
                        transmute::<&mut Repr, &mut HeapStr>(this)
                    };
                    repr.dealloc();
                }
                KIND_INTERNED => {
                    let repr = unsafe {
                        // SAFETY: We just checked the discriminant to make sure
                        // we're heap allocated
                        transmute::<&mut Repr, &mut Interned>(this)
                    };
                    repr.dealloc();
                }
                _ => unsafe { debug_unreachable!("Invalid kind in Repr::drop()") },
            }
        }
    }
}
