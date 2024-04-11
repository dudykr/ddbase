use crate::{Atom, AtomStore};

fn store_with_atoms(texts: Vec<&str>) -> (AtomStore, Vec<Atom>) {
    let mut store = AtomStore::default();

    let atoms = { texts.into_iter().map(|text| store.atom(text)).collect() };

    (store, atoms)
}

#[test]
fn simple_usage() {
    let (s, atoms) = store_with_atoms(vec!["Hello, world!", "Hello, world!"]);

    drop(s);

    let a1 = atoms[0].clone();

    let a2 = atoms[1].clone();

    assert_eq!(a1.unsafe_data, a2.unsafe_data);
}

#[test]
fn eager_drop() {
    let (_, atoms1) = store_with_atoms(vec!["Hello, world!!!!"]);
    let (_, atoms2) = store_with_atoms(vec!["Hello, world!!!!"]);

    dbg!(&atoms1);
    dbg!(&atoms2);

    let a1 = atoms1[0].clone();
    let a2 = atoms2[0].clone();

    assert_ne!(
        a1.unsafe_data, a2.unsafe_data,
        "Different stores should have different addresses"
    );
    assert_eq!(a1.get_hash(), a2.get_hash(), "Same string should be equal");
    assert_eq!(a1, a2, "Same string should be equal");
}

#[test]
fn store_multiple() {
    let (_s1, atoms1) = store_with_atoms(vec!["Hello, world!!!!"]);
    let (_s2, atoms2) = store_with_atoms(vec!["Hello, world!!!!"]);

    let a1 = atoms1[0].clone();
    let a2 = atoms2[0].clone();

    assert_ne!(
        a1.unsafe_data, a2.unsafe_data,
        "Different stores should have different addresses"
    );
    assert_eq!(a1.get_hash(), a2.get_hash(), "Same string should be equal");
    assert_eq!(a1, a2, "Same string should be equal");
}

#[test]
fn store_merge_two() {
    let (mut s1, atoms1) = store_with_atoms(vec!["Hello, world!!!!"]);
    let (s2, atoms2) = store_with_atoms(vec!["Hello, world!!!!"]);

    let a1 = atoms1[0].clone();
    let a2 = atoms2[0].clone();
    assert_eq!(a1, a2);
    assert!(!a1.simple_eq(&a2).unwrap_or_default());

    s1.merge(s2);

    let a3 = s1.atom("Hello, world!!!!");

    assert_eq!(
        a1.unsafe_data, a3.unsafe_data,
        "Merged store should give same address as `self`"
    );
    assert_eq!(
        a1.get_hash(),
        a3.get_hash(),
        "Merged store should give same hash as `self`"
    );
    assert_ne!(
        a2.unsafe_data, a3.unsafe_data,
        "Merged store should give different address as `other`"
    );

    assert!(a1.simple_eq(&a3).unwrap_or_default());
    assert!(a2.simple_eq(&a3).unwrap_or_default());
}

#[test]
fn store_merge_many_1() {
    let (mut s1, atoms1) = store_with_atoms(vec!["Hello, world!!!!"]);
    let (s2, atoms2) = store_with_atoms(vec!["Hello, world!!!!"]);
    let (s3, atoms3) = store_with_atoms(vec!["Hi!"]);

    let a1 = atoms1[0].clone();
    let a2 = atoms2[0].clone();
    let a3 = atoms3[0].clone();

    assert_eq!(a1, a2);
    assert_eq!(a1.simple_eq(&a2), None, "Same string, but different stores");
    assert_ne!(a1, a3);
    assert_eq!(a1.simple_eq(&a3), Some(false));
    assert_ne!(a2, a3);
    assert_eq!(a2.simple_eq(&a3), Some(false));

    s1.merge(s2);
    s1.merge(s3);

    let a4 = s1.atom("Hello, world!!!!");

    assert_eq!(
        a1.unsafe_data, a4.unsafe_data,
        "Merged store should give same address as `self`"
    );
    assert_eq!(
        a1.get_hash(),
        a4.get_hash(),
        "Merged store should give same hash as `self`"
    );
    assert_ne!(
        a2.unsafe_data, a4.unsafe_data,
        "Merged store should give different address as `other`"
    );
    assert_ne!(
        a3.unsafe_data, a4.unsafe_data,
        "Merged store should give different address as `other`"
    );

    assert_eq!(a1.simple_eq(&a4), Some(true));
    assert_eq!(a2.simple_eq(&a4), Some(true));
    assert_eq!(a3.simple_eq(&a4), Some(false));

    assert_eq!(a1, a4);
    assert_eq!(a2, a4);
    assert_ne!(a3, a4);
}
