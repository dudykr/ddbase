use std::{
    borrow::Cow,
    hash::BuildHasherDefault,
    ptr::null_mut,
    sync::{atomic::AtomicPtr, Arc, Weak},
};

use dashmap::DashMap;
use once_cell::sync::Lazy;
use rustc_hash::FxHasher;
use smallvec::SmallVec;

use crate::{
    dynamic::{new_atom, Entry, Storage},
    Atom,
};

#[derive(Default)]
struct GlobalData {
    data: DashMap<u32, SmallVec<[Weak<Entry>; 4]>, BuildHasherDefault<FxHasher>>,
}

impl Storage for &'_ GlobalData {
    fn insert_entry(self, text: Cow<str>, hash: u32) -> Arc<Entry> {
        let mut entries = self.data.entry(hash).or_insert_with(Default::default);

        // TODO(kdy1): This is extermely slow
        let existing = entries.iter().find_map(|entry| {
            let entry = entry.upgrade()?;

            if entry.hash == hash && *entry.string == text {
                Some(entry)
            } else {
                None
            }
        });

        match existing {
            Some(e) => e,
            None => {
                let e = Arc::new(Entry {
                    string: text.into_owned().into_boxed_str(),
                    hash,
                    store_id: None,
                    alias: AtomicPtr::new(null_mut()),
                });

                entries.push(Arc::downgrade(&e));

                e
            }
        }
    }
}

fn atom(text: Cow<str>) -> Atom {
    static GLOBAL_DATA: Lazy<GlobalData> = Lazy::new(Default::default);

    new_atom(&*GLOBAL_DATA, text)
}

macro_rules! direct_from_impl {
    ($T:ty) => {
        impl From<$T> for Atom {
            fn from(s: $T) -> Self {
                atom(s.into())
            }
        }
    };
}

direct_from_impl!(&'_ str);
direct_from_impl!(Cow<'_, str>);
direct_from_impl!(String);

impl From<Box<str>> for crate::Atom {
    fn from(s: Box<str>) -> Self {
        atom(Cow::Owned(String::from(s)))
    }
}
