use bytes::Bytes;

/// [str], but backed by [Bytes].
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Hash, Default)]
pub struct BytesStr {
    bytes: Bytes,
}
