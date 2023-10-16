//! See [Atom] for more information.

use std::{
    fmt::{Debug, Display},
    hash::Hash,
    mem::forget,
    ops::Deref,
    ptr::NonNull,
    sync::{atomic::Ordering::SeqCst, Arc},
};

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
    unsafe_data: NonNull<Entry>,
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

impl Atom {
    #[inline]
    fn is_dynamic(&self) -> bool {
        true
    }

    fn get_hash(&self) -> u32 {
        if self.is_dynamic() {
            unsafe { self.unsafe_data.as_ref() }.hash
        } else {
            0
        }
    }

    #[inline]
    fn as_str(&self) -> &str {
        unsafe { self.unsafe_data.as_ref() }.string.as_ref()
    }

    #[inline(never)]
    fn fast_eq(&self, other: &Self) -> Option<bool> {
        if self.unsafe_data == other.unsafe_data {
            return Some(true);
        }

        let te = unsafe { self.unsafe_data.as_ref() };
        let oe = unsafe { other.unsafe_data.as_ref() };

        if te.hash != oe.hash {
            return Some(false);
        }

        // This is slow, but we don't reach here in most cases

        if let Some(other_alias) = NonNull::new(oe.alias.load(SeqCst)) {
            if let Some(result) = self.fast_eq(&Atom::from_alias(other_alias)) {
                return Some(result);
            }
        }

        if let Some(self_alias) = NonNull::new(te.alias.load(SeqCst)) {
            if let Some(result) = other.fast_eq(&Atom::from_alias(self_alias)) {
                return Some(result);
            }
        }

        None
    }
}

impl PartialEq for Atom {
    #[inline]
    fn eq(&self, other: &Self) -> bool {
        if let Some(result) = self.fast_eq(other) {
            return result;
        }

        let te = unsafe { self.unsafe_data.as_ref() };
        let oe = unsafe { other.unsafe_data.as_ref() };

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

        // If the store is different, the string may be the same, even though the
        // `unsafe_data` is different
        te.string == oe.string
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
            unsafe { drop_slow(Entry::restore_arc(self.unsafe_data)) }
        }

        #[cold]
        #[inline(never)]
        fn drop_slow(arc: Arc<Entry>) {
            dbg!(Arc::strong_count(&arc));
            drop(arc);
        }
    }
}

impl Clone for Atom {
    fn clone(&self) -> Self {
        Self::from_alias(self.unsafe_data)
    }
}

impl Atom {
    pub(crate) fn from_alias(alias: NonNull<Entry>) -> Self {
        if true {
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
