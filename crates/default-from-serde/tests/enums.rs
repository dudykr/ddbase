use derive_default_from_serde::SerdeDefault;
use serde::Deserialize;

#[derive(SerdeDefault, Deserialize)]
pub enum Enum1 {}
