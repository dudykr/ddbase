//! See [Atom] for more information.

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    mem::{self, forget},
    num::NonZeroU64,
    ops::Deref,
    slice,
    sync::atomic::Ordering::SeqCst,
};

use debug_unreachable::debug_unreachable;
use once_cell::sync::Lazy;

use crate::dynamic::Entry;
pub use crate::{dynamic::AtomStore, global_store::*};

mod dynamic;
mod global_store;
#[cfg(test)]
mod tests;

/// An atom is an immutable string that is stored in some [AtomStore].
///
///
/// # Features
///
/// ## Fast equality check (in most cases)
///
/// Equality comparison is O(1). If two atoms are from the same store, or
/// they are from different stores but they are [`AtomStore::merge`]d, they are
/// compared by numeric equality.
///
///
/// ## Fast [Hash] implementation
///
///
///
/// ## Lock-free creation
///
/// - Note: This applies if you create atoms via [AtomStore]. If you create
///   atoms via global APIs, this does not apply.
///
/// ## Lock-free drop
///
/// [Drop] does not lock any mutex.
///
/// ## Small size (One `u64`)
///
/// ```rust
/// # use std::mem::size_of;
/// use hstr::Atom;
/// assert!(size_of::<Atom>() == size_of::<u64>());
/// assert!(size_of::<Option<Atom>>() == size_of::<u64>());
/// ````
///
/// ## Small strings as inline data
///
/// # Creating atoms
///
/// If you are working on a module which creates lots of [Atom]s, you are
/// recommended to use [AtomStore] API because it's faster. But if you are not,
/// you can use global APIs for convenience.

pub struct Atom {
    // If this Atom is a dynamic one, this is *const Entry
    unsafe_data: NonZeroU64,
}

impl Default for Atom {
    #[inline(never)]
    fn default() -> Self {
        static EMPTY: Lazy<Atom> = Lazy::new(|| Atom::from(""));
        EMPTY.clone()
    }
}

/// Immutable, so it's safe to be shared between threads
unsafe impl Send for Atom {}

/// Immutable, so it's safe to be shared between threads
unsafe impl Sync for Atom {}

impl Display for Atom {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self.as_str(), f)
    }
}

impl Debug for Atom {
    #[inline]
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Debug::fmt(self.as_str(), f)
    }
}

const DYNAMIC_TAG: u8 = 0b_00;
const INLINE_TAG: u8 = 0b_01; // len in upper nybble
const STATIC_TAG: u8 = 0b_10;
const TAG_MASK: u64 = 0b_11;
const LEN_OFFSET: u64 = 4;
const LEN_MASK: u64 = 0xf0;

const MAX_INLINE_LEN: usize = 7;
// const STATIC_SHIFT_BITS: usize = 32;

impl Atom {
    #[inline(always)]
    fn tag(&self) -> u8 {
        (self.unsafe_data.get() & TAG_MASK) as u8
    }

    /// Return true if this is a dynamic Atom.
    #[inline(always)]
    fn is_dynamic(&self) -> bool {
        self.tag() == DYNAMIC_TAG
    }
}

impl Atom {
    fn from_mutated_str<F: FnOnce(&mut str)>(s: &str, f: F) -> Self {
        let mut buffer = mem::MaybeUninit::<[u8; 64]>::uninit();
        let buffer = unsafe { &mut *buffer.as_mut_ptr() };

        if let Some(buffer_prefix) = buffer.get_mut(..s.len()) {
            buffer_prefix.copy_from_slice(s.as_bytes());
            let as_str = unsafe { ::std::str::from_utf8_unchecked_mut(buffer_prefix) };
            f(as_str);
            Atom::from(&*as_str)
        } else {
            let mut string = s.to_owned();
            f(&mut string);
            Atom::from(string)
        }
    }

    /// Like [`to_ascii_uppercase`].
    ///
    /// [`to_ascii_uppercase`]: https://doc.rust-lang.org/std/ascii/trait.AsciiExt.html#tymethod.to_ascii_uppercase
    pub fn to_ascii_uppercase(&self) -> Self {
        for (i, b) in self.bytes().enumerate() {
            if let b'a'..=b'z' = b {
                return Atom::from_mutated_str(self, |s| s[i..].make_ascii_uppercase());
            }
        }
        self.clone()
    }

    /// Like [`to_ascii_lowercase`].
    ///
    /// [`to_ascii_lowercase`]: https://doc.rust-lang.org/std/ascii/trait.AsciiExt.html#tymethod.to_ascii_lowercase
    pub fn to_ascii_lowercase(&self) -> Self {
        for (i, b) in self.bytes().enumerate() {
            if let b'A'..=b'Z' = b {
                return Atom::from_mutated_str(self, |s| s[i..].make_ascii_lowercase());
            }
        }
        self.clone()
    }
}

impl Atom {
    #[inline]
    fn get_hash(&self) -> u32 {
        match self.tag() {
            DYNAMIC_TAG => unsafe { Entry::deref_from(self.unsafe_data) }.hash,
            STATIC_TAG => {
                todo!("static hash")
            }
            INLINE_TAG => {
                let data = self.unsafe_data.get();
                // This may or may not be great...
                ((data >> 32) ^ data) as u32
            }
            _ => unsafe { debug_unreachable!() },
        }
    }

    #[inline]
    fn as_str(&self) -> &str {
        match self.tag() {
            DYNAMIC_TAG => unsafe { Entry::deref_from(self.unsafe_data) }
                .string
                .as_ref(),
            STATIC_TAG => {
                todo!("static as_str")
            }
            INLINE_TAG => {
                let len = (self.unsafe_data.get() & LEN_MASK) >> LEN_OFFSET;
                let src = inline_atom_slice(&self.unsafe_data);
                unsafe { std::str::from_utf8_unchecked(&src[..(len as usize)]) }
            }
            _ => unsafe { debug_unreachable!() },
        }
    }

    #[inline]
    fn simple_eq(&self, other: &Self) -> Option<bool> {
        if self.unsafe_data == other.unsafe_data {
            return Some(true);
        }

        // If one is inline and the other is not, the length is different.
        // If one is static and the other is not, it's different.
        if self.tag() != other.tag() {
            return Some(false);
        }

        if self.get_hash() != other.get_hash() {
            return Some(false);
        }

        self.simple_eq_slow(other)
    }

    #[inline(never)]
    fn simple_eq_slow(&self, other: &Self) -> Option<bool> {
        if self.is_dynamic() {
            // This is slow, but we don't reach here in most cases
            let te = unsafe { Entry::deref_from(self.unsafe_data) };

            if let Some(self_alias) = NonZeroU64::new(te.alias.load(SeqCst)) {
                if let Some(result) = other.simple_eq(&Atom::from_alias(self_alias)) {
                    return Some(result);
                }
            }
        }

        if other.is_dynamic() {
            let oe = unsafe { Entry::deref_from(other.unsafe_data) };
            if let Some(other_alias) = NonZeroU64::new(oe.alias.load(SeqCst)) {
                if let Some(result) = self.simple_eq(&Atom::from_alias(other_alias)) {
                    return Some(result);
                }
            }
        }

        None
    }
}

impl PartialEq for Atom {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        if let Some(result) = self.simple_eq(other) {
            return result;
        }

        if self.is_dynamic() && other.is_dynamic() {
            let te = unsafe { Entry::deref_from(self.unsafe_data) };
            let oe = unsafe { Entry::deref_from(other.unsafe_data) };

            // If the store is the same, the same string has same `unsafe_data``
            match (&te.store_id, &oe.store_id) {
                (Some(this_store), Some(other_store)) => {
                    if this_store == other_store {
                        return false;
                    }
                }
                (None, None) => {
                    return false;
                }
                _ => {}
            }
        }

        // If the store is different, the string may be the same, even though the
        // `unsafe_data` is different
        self.as_str() == other.as_str()
    }
}

impl Eq for Atom {}

impl Hash for Atom {
    #[inline]
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        state.write_u32(self.get_hash());
    }
}

impl Drop for Atom {
    #[inline]
    fn drop(&mut self) {
        if self.is_dynamic() {
            unsafe { drop(Entry::restore_arc(self.unsafe_data)) }
        }
    }
}

impl Clone for Atom {
    #[inline]
    fn clone(&self) -> Self {
        Self::from_alias(self.unsafe_data)
    }
}

impl Atom {
    #[inline]
    pub(crate) fn from_alias(alias: NonZeroU64) -> Self {
        if (alias.get() & TAG_MASK) as u8 == DYNAMIC_TAG {
            unsafe {
                let arc = Entry::restore_arc(alias);
                forget(arc.clone());
                forget(arc);
            }
        }

        Self { unsafe_data: alias }
    }
}

impl Deref for Atom {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl AsRef<str> for Atom {
    #[inline]
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl PartialEq<str> for Atom {
    #[inline]
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

#[inline(always)]
fn inline_atom_slice(x: &NonZeroU64) -> &[u8] {
    unsafe {
        let x: *const NonZeroU64 = x;
        let mut data = x as *const u8;
        // All except the lowest byte, which is first in little-endian, last in
        // big-endian.
        if cfg!(target_endian = "little") {
            data = data.offset(1);
        }
        let len = 7;
        slice::from_raw_parts(data, len)
    }
}

#[inline(always)]
fn inline_atom_slice_mut(x: &mut u64) -> &mut [u8] {
    unsafe {
        let x: *mut u64 = x;
        let mut data = x as *mut u8;
        // All except the lowest byte, which is first in little-endian, last in
        // big-endian.
        if cfg!(target_endian = "little") {
            data = data.offset(1);
        }
        let len = 7;
        slice::from_raw_parts_mut(data, len)
    }
}
