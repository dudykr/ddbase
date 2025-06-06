use bytes::Bytes;

/// [str], but backed by [Bytes].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BytesStr {
    bytes: Bytes,
}
