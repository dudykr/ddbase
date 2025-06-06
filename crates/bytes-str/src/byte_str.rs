use std::{
    borrow::{Borrow, Cow},
    cmp::Ordering,
    ffi::OsStr,
    fmt::{self, Debug, Display},
    hash::{Hash, Hasher},
    ops::{Deref, Index},
    path::Path,
    slice::SliceIndex,
    str::Utf8Error,
};

use bytes::Bytes;

use crate::BytesString;

/// A reference-counted `str` backed by [Bytes].
///
/// Clone is cheap thanks to [Bytes].
///
///
/// # Features
///
/// ## `rkyv`
///
/// If the `rkyv` feature is enabled, the [BytesStr] type will be
/// [rkyv::Archive], [rkyv::Serialize], and [rkyv::Deserialize].
///
///
/// ## `serde`
///
/// If the `serde` feature is enabled, the [BytesStr] type will be
/// [serde::Serialize] and [serde::Deserialize].
///
/// The [BytesStr] type will be serialized as a [str] type.
#[derive(Clone, Default, PartialEq, Eq)]
#[cfg_attr(
    feature = "rkyv",
    derive(rkyv::Archive, rkyv::Serialize, rkyv::Deserialize)
)]
pub struct BytesStr {
    pub(crate) bytes: Bytes,
}

impl BytesStr {
    /// Creates a new empty BytesStr.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    ///
    /// let s = BytesStr::new();
    ///
    /// assert_eq!(s.as_str(), "");
    /// ```
    pub fn new() -> Self {
        Self {
            bytes: Bytes::new(),
        }
    }

    /// Creates a new BytesStr from a static string.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    pub fn from_static(bytes: &'static str) -> Self {
        Self {
            bytes: Bytes::from_static(bytes.as_bytes()),
        }
    }

    /// Creates a new BytesStr from a [Bytes].
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    /// use bytes::Bytes;
    ///
    /// let s = BytesStr::from_utf8(Bytes::from_static(b"hello")).unwrap();
    ///
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    pub fn from_utf8(bytes: Bytes) -> Result<Self, Utf8Error> {
        std::str::from_utf8(&bytes)?;

        Ok(Self { bytes })
    }

    /// Creates a new BytesStr from a [Vec<u8>].
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    /// use bytes::Bytes;
    ///
    /// let s = BytesStr::from_utf8_vec(b"hello".to_vec()).unwrap();
    ///
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    pub fn from_utf8_vec(bytes: Vec<u8>) -> Result<Self, Utf8Error> {
        std::str::from_utf8(&bytes)?;

        Ok(Self {
            bytes: Bytes::from(bytes),
        })
    }

    /// Creates a new BytesStr from a [Bytes] without checking if the bytes
    /// are valid UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are valid
    /// UTF-8. If the bytes are not valid UTF-8, the resulting BytesStr will
    pub unsafe fn from_utf8_unchecked(bytes: Bytes) -> Self {
        Self { bytes }
    }

    /// Creates a new BytesStr from a [Vec<u8>] without checking if the bytes
    /// are valid UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are valid
    /// UTF-8. If the bytes are not valid UTF-8, the resulting BytesStr will
    /// be invalid.
    pub unsafe fn from_utf8_vec_unchecked(bytes: Vec<u8>) -> Self {
        Self::from_utf8_unchecked(Bytes::from(bytes))
    }

    /// Creates a new BytesStr from a [Bytes].
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    /// use bytes::Bytes;
    ///     
    /// let s = BytesStr::from_utf8_slice(b"hello").unwrap();
    ///
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    pub fn from_utf8_slice(bytes: &[u8]) -> Result<Self, Utf8Error> {
        std::str::from_utf8(bytes)?;

        Ok(Self {
            bytes: Bytes::copy_from_slice(bytes),
        })
    }

    /// Creates a new BytesStr from a [Bytes] without checking if the bytes
    /// are valid UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are valid
    /// UTF-8. If the bytes are not valid UTF-8, the resulting BytesStr will
    /// be invalid.
    pub unsafe fn from_utf8_slice_unchecked(bytes: &[u8]) -> Self {
        Self {
            bytes: Bytes::copy_from_slice(bytes),
        }
    }

    /// Creates a new BytesStr from a static UTF-8 slice.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesStr;
    ///     
    /// let s = BytesStr::from_static_utf8_slice(b"hello").unwrap();
    ///
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    pub fn from_static_utf8_slice(bytes: &'static [u8]) -> Result<Self, Utf8Error> {
        std::str::from_utf8(bytes)?;

        Ok(Self {
            bytes: Bytes::from_static(bytes),
        })
    }

    /// Creates a new BytesStr from a static UTF-8 slice without checking if the
    /// bytes are valid UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are valid
    /// UTF-8. If the bytes are not valid UTF-8, the resulting BytesStr will
    /// be invalid.
    pub unsafe fn from_static_utf8_slice_unchecked(bytes: &'static [u8]) -> Self {
        Self {
            bytes: Bytes::from_static(bytes),
        }
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

impl Borrow<str> for BytesStr {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl Debug for BytesStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(self.as_str(), f)
    }
}

impl Display for BytesStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(self.as_str(), f)
    }
}

impl Extend<BytesStr> for BytesString {
    fn extend<T: IntoIterator<Item = BytesStr>>(&mut self, iter: T) {
        self.bytes.extend(iter.into_iter().map(|s| s.bytes));
    }
}

impl<I> Index<I> for BytesStr
where
    I: SliceIndex<str>,
{
    type Output = I::Output;

    fn index(&self, index: I) -> &Self::Output {
        self.as_str().index(index)
    }
}

impl PartialEq<str> for BytesStr {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&'_ str> for BytesStr {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<Cow<'_, str>> for BytesStr {
    fn eq(&self, other: &Cow<'_, str>) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<BytesStr> for str {
    fn eq(&self, other: &BytesStr) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<BytesStr> for &'_ str {
    fn eq(&self, other: &BytesStr) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<BytesStr> for Bytes {
    fn eq(&self, other: &BytesStr) -> bool {
        self == other.as_bytes()
    }
}

impl PartialEq<String> for BytesStr {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<BytesStr> for String {
    fn eq(&self, other: &BytesStr) -> bool {
        self == other.as_str()
    }
}

impl Ord for BytesStr {
    fn cmp(&self, other: &Self) -> Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for BytesStr {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// This produces the same hash as [str]
impl Hash for BytesStr {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl TryFrom<&'static [u8]> for BytesStr {
    type Error = Utf8Error;

    fn try_from(value: &'static [u8]) -> Result<Self, Self::Error> {
        Self::from_static_utf8_slice(value)
    }
}

#[cfg(feature = "serde")]
mod serde_impl {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::*;

    impl<'de> Deserialize<'de> for BytesStr {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
        {
            let s = String::deserialize(deserializer)?;
            Ok(Self::from(s))
        }
    }

    impl Serialize for BytesStr {
        fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.serialize_str(self.as_str())
        }
    }
}
