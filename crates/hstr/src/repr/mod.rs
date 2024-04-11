use self::nonmax_u8::NonMaxU8;

mod heap;
mod inline;
mod interned;
mod nonmax_u8;
mod static_ref;

#[repr(C)]
pub struct Repr(
    // We have a pointer in the repesentation to properly carry provenance
    *const u8,
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

    fn len(&self) -> usize {}

    fn as_str(&self) -> &str {}

    #[inline]
    fn kind(&self) -> u8 {
        self.last_byte() & KIND_MASK
    }

    fn last_byte(&self) -> u8 {
        cfg_if::cfg_if! {
            if #[cfg(target_pointer_width = "64")] {
                let last_byte = self.4;
            } else if #[cfg(target_pointer_width = "32")] {
                let last_byte = self.3;
            } else {
                compile_error!("Unsupported target_pointer_width");
            }
        };
        last_byte as u8
    }
}

static_assertions::assert_eq_size!(Repr, Option<Repr>, [usize; 2]);
