use tagged_union::TaggedUnion;

#[derive(TaggedUnion)]
enum Pat {
    Ident(Ident),
    ArrayPat(ArrayPat),
    ObjectPat(ObjectPat),
    AssignPat(AssignPat),
}

#[derive(TaggedUnion)]
enum AssignTargetPat {
    ArrayPat(ArrayPat),
    ObjectPat(ObjectPat),
}

pub struct Ident {}

pub struct AssignPat {}

pub struct ArrayPat {}

pub struct ObjectPat {}
