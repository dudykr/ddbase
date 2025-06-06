use bytes::BytesMut;

/// [String] but backed by a [BytesMut]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteString {
    bytes: BytesMut,
}
