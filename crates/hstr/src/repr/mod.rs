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

static_assertions::assert_eq_size!(Repr, [usize; 2]);
