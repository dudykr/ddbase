# default-from-serde

Derive `Default` using `serde::Deserialize`!
No mismatch between `Default` and `Deserialize` anymore!

# Usage

`Cargo.toml`:

```toml
default-from-serde = "0.1"
```

See [docs.rs](https://docs.rs/default-from-serde) for the Rust code.

# License

APACHE-2.0.

Some source code are copied from `serde_json`.
This library is pracrically deserialize using `serde_json` with `{}`.
