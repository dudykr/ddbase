use derive_default_from_serde::SerdeDefault;
use serde::Deserialize;

#[derive(SerdeDefault, Deserialize)]
pub struct Struct1 {}
