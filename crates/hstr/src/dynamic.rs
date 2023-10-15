use std::{
    borrow::Cow,
    collections::HashMap,
    fmt::Debug,
    hash::{Hash, Hasher},
    mem::forget,
    num::{NonZeroU32, NonZeroU64},
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering::SeqCst},
        Arc,
    },
};

use rustc_hash::{FxHashMap, FxHasher};
use smallvec::SmallVec;

use crate::{no_inline_clone, Atom};

#[derive(Debug)]
pub(crate) struct Entry {
    pub string: Box<str>,
    pub hash: u32,
    /// store id
    pub store_id: Option<NonZeroU32>,

    pub alias: AtomicU64,
}

impl Entry {
    pub fn cast(v: NonZeroU64) -> *const Entry {
        v.get() as *const Entry
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

    /// Exists just to prevent dropping of the entries
    pub(crate) extras: Vec<Arc<Entry>>,
}

impl Default for AtomStore {
    fn default() -> Self {
        static ATOM_STORE_ID: AtomicU32 = AtomicU32::new(1);

        Self {
            id: Some(unsafe { NonZeroU32::new_unchecked(ATOM_STORE_ID.fetch_add(1, SeqCst)) }),
            data: HashMap::with_capacity_and_hasher(64, Default::default()),
            extras: Default::default(),
        }
    }
}

impl AtomStore {
    ///
    pub fn merge(&mut self, other: AtomStore) {
        for (_, entries) in other.data {
            for entry in entries {
                let cur_entry = self.insert_entry(Cow::Borrowed(&entry.string), entry.hash);

                let ptr = Arc::as_ptr(&cur_entry);
                let unsafe_data = ptr as u64;

                entry.alias.store(unsafe_data, SeqCst);

                self.extras.push(entry);

                // We
            }
        }
    }

    #[inline(always)]
    pub fn atom<'a>(&mut self, text: impl Into<Cow<'a, str>>) -> Atom {
        self.atom_inner(text.into())
    }

    #[inline(never)]
    fn atom_inner(&mut self, text: Cow<str>) -> Atom {
        new_atom(self, text)
    }
}

pub(crate) fn new_atom<S>(storage: S, text: Cow<str>) -> Atom
where
    S: Storage,
{
    let hash = calc_hash(&text);
    let entry = storage.insert_entry(text, hash);
    let ptr = Arc::as_ptr(&entry);
    let data = ptr as u64;

    forget(entry);

    // debug_assert!(0 == data & TAG_MASK);
    Atom {
        unsafe_data: unsafe {
            // Safety: The address of a Arc is non-zero
            NonZeroU64::new_unchecked(data)
        },
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
                .find(|entry| entry.hash == hash && &*entry.string == text)
                .cloned()
        });

        match existing {
            Some(e) => e,
            None => {
                let e = no_inline_wrap(|| {
                    Arc::new(Entry {
                        string: text.into_owned().into_boxed_str(),
                        hash,
                        store_id,
                        alias: AtomicU64::new(0),
                    })
                });
                let v = no_inline_clone(&e);

                entries.push(e);

                v
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
