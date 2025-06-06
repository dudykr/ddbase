use std::{
    borrow::Borrow,
    ops::{Deref, DerefMut},
};

use bytes::{Bytes, BytesMut};

/// [String] but backed by a [BytesMut]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ByteString {
    bytes: BytesMut,
}

impl Deref for ByteString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }
}

impl DerefMut for ByteString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::str::from_utf8_unchecked_mut(&mut self.bytes) }
    }
}

impl AsRef<str> for ByteString {
    fn as_ref(&self) -> &str {
        self.deref()
    }
}

impl Borrow<str> for ByteString {
    fn borrow(&self) -> &str {
        self.as_ref()
    }
}

impl From<String> for ByteString {
    fn from(s: String) -> Self {
        Self {
            bytes: Bytes::from(s.into_bytes()).into(),
        }
    }
}

impl From<&str> for ByteString {
    fn from(s: &str) -> Self {
        Self {
            bytes: BytesMut::from(s),
        }
    }
}
