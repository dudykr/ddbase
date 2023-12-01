use derive_default_from_serde::SerdeDefault;
use serde_derive::Deserialize;

#[derive(SerdeDefault, Deserialize)]
struct Struct1 {
    #[serde(default)]
    field: String,

    #[serde(default = "true_by_default")]
    custom_default: bool,
}

fn true_by_default() -> bool {
    true
}

#[test]
fn test() {
    let s = Struct1::default();

    assert_eq!(s.field, String::default());
    assert!(s.custom_default);
}
