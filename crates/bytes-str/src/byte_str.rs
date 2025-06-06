use std::{borrow::Borrow, ops::Deref};

use bytes::Bytes;

use crate::BytesString;

/// [str], but backed by [Bytes].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BytesStr {
    pub(crate) bytes: Bytes,
}

impl Deref for BytesStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl AsRef<str> for BytesStr {
    fn as_ref(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }
}

impl Borrow<str> for BytesStr {
    fn borrow(&self) -> &str {
        self.as_ref()
    }
}

impl From<String> for BytesStr {
    fn from(s: String) -> Self {
        Self {
            bytes: Bytes::from(s),
        }
    }
}

impl From<&'static str> for BytesStr {
    fn from(s: &'static str) -> Self {
        Self {
            bytes: Bytes::from_static(s.as_bytes()),
        }
    }
}

impl From<BytesStr> for BytesString {
    fn from(s: BytesStr) -> Self {
        Self {
            bytes: s.bytes.into(),
        }
    }
}

impl From<BytesString> for BytesStr {
    fn from(s: BytesString) -> Self {
        Self {
            bytes: s.bytes.into(),
        }
    }
}
