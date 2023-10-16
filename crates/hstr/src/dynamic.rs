use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::Debug,
    hash::{Hash, Hasher},
    num::{NonZeroU32, NonZeroU64},
    ptr::NonNull,
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering::SeqCst},
        Arc,
    },
};

use rustc_hash::{FxHashMap, FxHasher};
use smallvec::SmallVec;

use crate::{Atom, INLINE_TAG, LEN_OFFSET, MAX_INLINE_LEN, TAG_MASK};

#[derive(Debug)]
pub(crate) struct Entry {
    pub string: Box<str>,
    pub hash: u32,
    /// store id
    pub store_id: Option<NonZeroU32>,

    pub alias: AtomicU64,
}

impl Entry {
    pub unsafe fn cast(ptr: NonZeroU64) -> *const Entry {
        ptr.get() as *const Entry
    }

    pub unsafe fn deref_from<'i>(ptr: NonZeroU64) -> &'i Entry {
        &*Self::cast(ptr)
    }

    pub unsafe fn restore_arc(v: NonZeroU64) -> Arc<Entry> {
        let ptr = v.get() as *const Entry;
        Arc::from_raw(ptr)
    }
}

/// A store that stores [Atom]s. Can be merged with other [AtomStore]s for
/// better performance.
///
///
/// # Merging [AtomStore]
#[derive(Debug)]
pub struct AtomStore {
    pub(crate) id: Option<NonZeroU32>,
    pub(crate) data: FxHashMap<u32, SmallVec<[Arc<Entry>; 4]>>,
}

impl Default for AtomStore {
    fn default() -> Self {
        static ATOM_STORE_ID: AtomicU32 = AtomicU32::new(1);

        Self {
            id: Some(unsafe { NonZeroU32::new_unchecked(ATOM_STORE_ID.fetch_add(1, SeqCst)) }),
            data: HashMap::with_capacity_and_hasher(64, Default::default()),
        }
    }
}

impl AtomStore {
    ///
    pub fn merge(&mut self, other: AtomStore) {
        for (_, entries) in other.data {
            for entry in entries {
                let cur_entry = self.insert_entry(Cow::Borrowed(&entry.string), entry.hash);

                let ptr = unsafe { NonNull::new_unchecked(Arc::as_ptr(&cur_entry) as *mut Entry) };

                entry.alias.store(ptr.as_ptr() as u64, SeqCst);
            }
        }
    }

    #[inline(always)]
    pub fn atom<'a>(&mut self, text: impl Into<Cow<'a, str>>) -> Atom {
        new_atom(self, text.into())
    }
}

/// This can create any kind of [Atom], although this lives in the `dynamic`
/// module.
pub(crate) fn new_atom<S>(storage: S, text: Cow<str>) -> Atom
where
    S: Storage,
{
    let len = text.len();

    if len < MAX_INLINE_LEN {
        let mut data: u64 = (INLINE_TAG as u64) | ((len as u64) << LEN_OFFSET);
        {
            let dest = inline_atom_slice_mut(&mut data);
            dest[..len].copy_from_slice(text.as_bytes())
        }
        return Atom {
            // INLINE_TAG ensures this is never zero
            unsafe_data: unsafe { NonZeroU64::new_unchecked(data) },
        };
    }

    let hash = calc_hash(&text);
    let entry = storage.insert_entry(text, hash);
    let entry = Arc::into_raw(entry);

    let ptr: NonNull<Entry> = unsafe {
        // Safety: Arc::into_raw returns a non-null pointer
        NonNull::new_unchecked(entry as *mut Entry)
    };
    let data = ptr.as_ptr() as u64;

    debug_assert!(0 == data & TAG_MASK);
    Atom {
        unsafe_data: unsafe { NonZeroU64::new_unchecked(data) },
    }
}

pub(crate) trait Storage {
    fn insert_entry(self, text: Cow<str>, hash: u32) -> Arc<Entry>;
}

impl Storage for &'_ mut AtomStore {
    #[inline(never)]
    fn insert_entry(self, text: Cow<str>, hash: u32) -> Arc<Entry> {
        let store_id = self.id;

        let entries = self.data.entry(hash).or_insert_with(Default::default);

        // TODO(kdy1): This is extermely slow
        let existing = no_inline_wrap(|| {
            if entries.len() == 1 {
                return Some(entries[0].clone());
            }

            entries
                .iter()
                .find(|entry| entry.hash == hash && *entry.string == text)
                .cloned()
        });

        match existing {
            Some(existing) => existing,
            None => {
                let e = no_inline_wrap(|| {
                    Arc::new(Entry {
                        string: text.into_owned().into_boxed_str(),
                        hash,
                        store_id,
                        alias: AtomicU64::new(0),
                    })
                });
                let new = e.clone();

                entries.push(e);

                new
            }
        }
    }
}

#[inline(never)]
fn calc_hash(text: &str) -> u32 {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish() as u32
}

#[inline(never)]
fn no_inline_wrap<F, Ret>(op: F) -> Ret
where
    F: FnOnce() -> Ret,
{
    op()
}
