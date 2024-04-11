use self::nonmax::NonMaxUsize;

mod heap;
mod inline;
mod interned;
mod nonmax;
mod static_ref;

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
    pub fn new_static(text: &'static str) -> Self {}

    #[inline]
    pub fn new_dynamic(text: &str) -> Self {}

    #[inline]
    pub fn new_interned(text: &str) -> Self {}

    fn len(&self) -> usize {}

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
