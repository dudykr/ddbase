use bytes::BytesMut;

/// [String] but backed by a [BytesMut]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ByteString {
    bytes: BytesMut,
}
