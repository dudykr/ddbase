use std::{borrow::Borrow, ffi::OsStr, ops::Deref, path::Path, str::Utf8Error};

use bytes::Bytes;

use crate::BytesString;

/// [str], but backed by [Bytes].
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BytesStr {
    pub(crate) bytes: Bytes,
}

impl BytesStr {
    pub fn new() -> Self {
        Self {
            bytes: Bytes::new(),
        }
    }

    pub fn from_static(bytes: &'static str) -> Self {
        Self {
            bytes: Bytes::from_static(bytes.as_bytes()),
        }
    }

    pub fn from_utf8(bytes: Bytes) -> Result<Self, Utf8Error> {
        std::str::from_utf8(&bytes)?;

        Ok(Self { bytes })
    }

    pub fn from_utf8_slice(bytes: &[u8]) -> Result<Self, Utf8Error> {
        std::str::from_utf8(bytes)?;

        Ok(Self {
            bytes: Bytes::copy_from_slice(bytes),
        })
    }

    pub fn from_static_utf8_slice(bytes: &'static [u8]) -> Result<Self, Utf8Error> {
        std::str::from_utf8(bytes)?;

        Ok(Self {
            bytes: Bytes::from_static(bytes),
        })
    }

    /// Returns a string slice containing the entire BytesStr.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    ///
    /// let s = BytesStr::from_static("hello");
    ///
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }
}

impl Deref for BytesStr {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl AsRef<str> for BytesStr {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for BytesStr {
    fn borrow(&self) -> &str {
        self.as_str()
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

impl AsRef<[u8]> for BytesStr {
    fn as_ref(&self) -> &[u8] {
        self.bytes.as_ref()
    }
}

impl AsRef<Bytes> for BytesStr {
    fn as_ref(&self) -> &Bytes {
        &self.bytes
    }
}

impl AsRef<OsStr> for BytesStr {
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.as_str())
    }
}

impl AsRef<Path> for BytesStr {
    fn as_ref(&self) -> &Path {
        Path::new(self.as_str())
    }
}
