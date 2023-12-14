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

struct Ident {}

struct AssignPat {}

struct ArrayPat {}

struct ObjectPat {}
