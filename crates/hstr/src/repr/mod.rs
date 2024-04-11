use self::nonmax_u8::NonMaxU8;

mod heap;
mod inline;
mod interned;
mod nonmax_u8;
mod static_ref;

#[repr(C)]
pub struct Repr(
    // We have a pointer in the repesentation to properly carry provenance
    *const (),
    // Then we need one `usize` (aka WORDs) of data
    // ...but we breakup into multiple pieces...
    #[cfg(target_pointer_width = "64")] u32,
    u16,
    u8,
    // ...so that the last byte can be a NonMax, which allows the compiler to see a niche value
    NonMaxU8,
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
    pub fn new_static(text: &'static str) -> Self {}

    #[inline]
    pub fn new_dynamic(text: &str) -> Self {}

    #[inline]
    pub fn new_interned(text: &str) -> Self {}
}

static_assertions::assert_eq_size!(Repr, Option<Repr>, [usize; 2]);
