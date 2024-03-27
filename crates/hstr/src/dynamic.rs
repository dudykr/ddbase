use std::{
    borrow::{Borrow, Cow},
    collections::HashSet,
    fmt::Debug,
    hash::{BuildHasherDefault, Hash, Hasher},
    num::{NonZeroU32, NonZeroU64},
    ops::Deref,
    ptr::NonNull,
    sync::{
        atomic::{AtomicU32, AtomicU64, Ordering::SeqCst},
        Arc,
    },
};

use rustc_hash::FxHasher;

use crate::{inline_atom_slice_mut, Atom, INLINE_TAG, LEN_OFFSET, MAX_INLINE_LEN, TAG_MASK};

#[derive(Debug)]
pub(crate) struct Entry {
    key: EntryKey<'static>,
    pub store_id: Option<NonZeroU32>,
    pub alias: AtomicU64,
}

impl Entry {
    pub fn string(&self) -> &str {
        &self.key.string
    }

    pub fn get_hash(&self) -> u64 {
        self.key.hash
    }

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

impl PartialEq for Entry {
    fn eq(&self, other: &Self) -> bool {
        // Assumption: `store_id` and `alias` don't matter for equality within a single
        // store (what we care about here).
        self.key == other.key
    }
}

impl Eq for Entry {}

impl Hash for Entry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.key.hash(state);
    }
}

/// The subset of Entry that's used for equality, internally used for lookups in
/// the HashSet.
#[derive(Debug)]
struct EntryKey<'a> {
    string: CowBoxStr<'a>,
    hash: u64,
}

impl<'a> Borrow<EntryKey<'a>> for Arc<Entry> {
    fn borrow(&self) -> &EntryKey<'a> {
        &self.key
    }
}

impl Hash for EntryKey<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // Assumption: type H is an EntryHasher
        state.write_u64(self.hash);
    }
}

impl PartialEq for EntryKey<'_> {
    fn eq(&self, other: &Self) -> bool {
        // do the cheaper hash comparison first
        self.hash == other.hash && self.string == other.string
    }
}

impl Eq for EntryKey<'_> {}

/// Roughly equivalent to a `Cow<'a, str>`, except that the owned representation
/// is `Box<str>` instead of `String`, which is slightly smaller (as it doesn't
/// store a capacity field).
#[derive(Debug)]
enum CowBoxStr<'a> {
    Owned(Box<str>),
    Borrowed(&'a str),
}

impl PartialEq for CowBoxStr<'_> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<'a> Deref for CowBoxStr<'a> {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Owned(s) => s,
            Self::Borrowed(s) => s,
        }
    }
}

impl<'a> From<Cow<'a, str>> for CowBoxStr<'a> {
    fn from(value: Cow<'a, str>) -> Self {
        match value {
            Cow::Owned(s) => Self::Owned(s.into_boxed_str()),
            Cow::Borrowed(s) => Self::Borrowed(s),
        }
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
    pub(crate) data: HashSet<Arc<Entry>, BuildEntryHasher>,
}

impl Default for AtomStore {
    fn default() -> Self {
        static ATOM_STORE_ID: AtomicU32 = AtomicU32::new(1);

        Self {
            id: Some(unsafe { NonZeroU32::new_unchecked(ATOM_STORE_ID.fetch_add(1, SeqCst)) }),
            data: HashSet::with_capacity_and_hasher(64, Default::default()),
        }
    }
}

impl AtomStore {
    ///
    pub fn merge(&mut self, other: AtomStore) {
        for entry in other.data {
            let cur_entry = self.insert_entry(Cow::Borrowed(entry.string()), entry.get_hash());

            let ptr = unsafe { NonNull::new_unchecked(Arc::as_ptr(&cur_entry) as *mut Entry) };

            entry.alias.store(ptr.as_ptr() as u64, SeqCst);
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
    fn insert_entry(self, text: Cow<str>, hash: u64) -> Arc<Entry>;
}

impl Storage for &'_ mut AtomStore {
    #[inline(never)]
    fn insert_entry(self, text: Cow<str>, hash: u64) -> Arc<Entry> {
        let store_id = self.id;
        let lookup_key = EntryKey {
            string: CowBoxStr::Borrowed(&text),
            hash,
        };

        if let Some(existing) = self.data.get(&lookup_key) {
            return existing.clone();
        }

        let new_entry = Arc::new(Entry {
            key: EntryKey {
                string: CowBoxStr::Owned(text.into_owned().into_boxed_str()),
                hash,
            },
            store_id,
            alias: AtomicU64::new(0),
        });
        let new_entry_ret = new_entry.clone();
        self.data.insert(new_entry);
        new_entry_ret
    }
}

#[inline(never)]
fn calc_hash(text: &str) -> u64 {
    let mut hasher = FxHasher::default();
    text.hash(&mut hasher);
    hasher.finish()
}

type BuildEntryHasher = BuildHasherDefault<EntryHasher>;

/// A "no-op" hasher for [Entry] that returns [Entry::hash]. The design is
/// inspired by the `nohash-hasher` crate.
///
/// Assumption: [Arc]'s implementation of [Hash] is a simple pass-through.
#[derive(Default)]
pub(crate) struct EntryHasher {
    hash: u64,
    #[cfg(debug_assertions)]
    write_called: bool,
}

impl Hasher for EntryHasher {
    fn finish(&self) -> u64 {
        #[cfg(debug_assertions)]
        debug_assert!(
            self.write_called,
            "EntryHasher expects write_u64 to have been called",
        );
        self.hash
    }

    fn write(&mut self, _bytes: &[u8]) {
        panic!("EntryHasher expects to be called with write_u64");
    }

    fn write_u64(&mut self, val: u64) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                !self.write_called,
                "EntryHasher expects write_u64 to be called only once",
            );
            self.write_called = true;
        }

        self.hash = val;
    }
}
