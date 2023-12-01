use derive_default_from_serde::SerdeDefault;
use serde_derive::Deserialize;

#[derive(SerdeDefault, Deserialize)]
enum Enum1 {
    A(Foo),
    B(Bar),
    C(Baz),
}

#[derive(Deserialize)]
struct Foo {}

#[derive(Deserialize)]
struct Bar {}

#[derive(Deserialize)]
struct Baz {}
