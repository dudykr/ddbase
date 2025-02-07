use shrink_to_fit::ShrinkToFit;

#[derive(Debug, ShrinkToFit)]
struct S {
    a: String,
    b: String,
}

#[test]
fn test_shrink_to_fit() {
    let mut s = S {
        a: String::with_capacity(100),
        b: String::with_capacity(100),
    };

    s.a.push('a');
    s.b.push('b');

    s.shrink_to_fit();

    assert_eq!(s.a.capacity(), 1);
    assert_eq!(s.b.capacity(), 1);
}

#[derive(Debug, ShrinkToFit)]
struct AutoDerefSpecialization {
    a: String,
    b: NotImplementShrinkToFit,
}

#[derive(Debug)]
struct NotImplementShrinkToFit;

#[test]
fn test_auto_deref_specialization() {
    let mut s = AutoDerefSpecialization {
        a: String::with_capacity(100),
        b: NotImplementShrinkToFit,
    };

    s.a.push('a');

    s.shrink_to_fit();

    assert_eq!(s.a.capacity(), 1);
}

#[test]
#[cfg(feature = "nightly")]
fn test_nightly_specialization() {
    let mut s = S {
        a: String::with_capacity(100),
        b: String::with_capacity(100),
    };

    s.a.push('a');

    let mut buf = vec![s];

    dbg!("before shrink_to_fit");
    ShrinkToFit::shrink_to_fit(&mut buf);
    dbg!("after shrink_to_fit");

    assert_eq!(buf.len(), 1);
    assert_eq!(buf[0].a.capacity(), 1);
    assert_eq!(buf[0].b.capacity(), 0);
}

#[deny(unused)]
mod helpers {
    pub use shrink_to_fit;
}

#[derive(Debug, ShrinkToFit)]
#[shrink_to_fit(crate = "crate::helpers::shrink_to_fit")]
struct ArgCheck {
    a: String,
    b: String,
}

#[test]
fn test_arg_check() {
    let mut s = ArgCheck {
        a: String::with_capacity(100),
        b: String::with_capacity(100),
    };

    s.a.push('a');

    s.shrink_to_fit();

    assert_eq!(s.a.capacity(), 1);
    assert_eq!(s.b.capacity(), 0);
}
