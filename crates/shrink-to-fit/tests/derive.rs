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
