use derive_default_from_serde::SerdeDefault;
use serde_derive::Deserialize;

#[derive(SerdeDefault, Deserialize)]
struct Struct1 {}
