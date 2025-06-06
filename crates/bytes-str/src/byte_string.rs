use std::{
    borrow::Borrow,
    ops::{Deref, DerefMut},
    str::Utf8Error,
};

use bytes::{Bytes, BytesMut};

/// [String] but backed by a [BytesMut]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BytesString {
    pub(crate) bytes: BytesMut,
}

impl BytesString {
    /// Returns a new, empty BytesString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::new();
    ///
    /// assert!(s.is_empty());
    /// ```
    pub fn new() -> Self {
        Self {
            bytes: BytesMut::new(),
        }
    }

    /// Returns a new, empty BytesString with the specified capacity.
    ///
    /// The capacity is the size of the internal buffer in bytes.
    ///
    /// The actual capacity may be larger than the specified capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::with_capacity(10);
    ///
    /// assert!(s.capacity() >= 10);
    /// ```
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: BytesMut::with_capacity(capacity),
        }
    }

    /// Returns the length of this String, in bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::from("hello");
    ///
    /// assert_eq!(s.len(), 5);
    /// ```
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns the capacity of this String, in bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::from("hello");
    ///
    /// assert!(s.capacity() >= 5);
    /// ```
    pub fn capacity(&self) -> usize {
        self.bytes.capacity()
    }

    /// Reserves the minimum capacity for exactly `additional` more bytes to be
    /// stored without reallocating.
    ///
    /// # Panics
    ///
    /// Panics if the new capacity overflows usize.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// s.reserve(10);
    ///
    /// assert!(s.capacity() >= 15);
    /// ```
    pub fn reserve(&mut self, additional: usize) {
        self.bytes.reserve(additional);
    }

    /// Splits the string into two at the given index.
    ///
    /// Returns a newly allocated String. `self` contains bytes at indices
    /// greater than `at`, and the returned string contains bytes at indices
    /// less than `at`.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// let other = s.split_off(2);
    ///
    /// assert_eq!(s, "he");
    /// assert_eq!(other, "llo");
    /// ```
    pub fn split_off(&mut self, at: usize) -> Self {
        Self {
            bytes: self.bytes.split_off(at),
        }
    }

    /// Returns a byte slice of this String’s contents.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::from("hello");
    ///
    /// assert_eq!(s.as_bytes(), b"hello");
    /// ```
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Returns true if the BytesString has a length of 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::new();
    ///
    /// assert!(s.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Truncates the BytesString to the specified length.
    ///
    /// If new_len is greater than or equal to the string’s current length, this
    /// has no effect.
    ///
    /// Note that this method has no effect on the allocated capacity of the
    /// string
    ///
    /// # Arguments
    ///
    /// * `new_len` - The new length of the BytesString
    ///
    /// # Panics
    ///
    /// Panics if new_len does not lie on a char boundary.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// s.truncate(3);
    ///
    /// assert_eq!(s, "hel");
    /// ```
    ///
    ///
    /// Shortens this String to the specified length.
    pub fn truncate(&mut self, new_len: usize) {
        if new_len <= self.len() {
            assert!(self.is_char_boundary(new_len));
            self.bytes.truncate(new_len);
        }
    }

    /// Clears the BytesString, removing all bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// s.clear();
    ///
    /// assert!(s.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Appends a character to the end of this BytesString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// s.push(' ');
    ///
    /// assert_eq!(s, "hello ");
    /// ```
    pub fn push(&mut self, ch: char) {
        let mut buf = [0; 4];
        let bytes = ch.encode_utf8(&mut buf);
        self.bytes.extend_from_slice(bytes.as_bytes());
    }

    /// Appends a string slice to the end of this BytesString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// s.push_str(" world");
    ///
    /// assert_eq!(s, "hello world");
    /// ```
    pub fn push_str(&mut self, s: &str) {
        self.bytes.extend_from_slice(s.as_bytes());
    }

    /// Returns a string slice containing the entire BytesString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::from("hello");
    ///
    /// assert_eq!(s.as_str(), "hello");
    /// ```
    pub fn as_str(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }

    /// Returns a mutable string slice containing the entire BytesString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let mut s = BytesString::from("hello");
    ///
    /// s.as_mut_str().make_ascii_uppercase();
    ///
    /// assert_eq!(s, "HELLO");
    /// ```
    pub fn as_mut_str(&mut self) -> &mut str {
        unsafe { std::str::from_utf8_unchecked_mut(&mut self.bytes) }
    }

    /// Converts the BytesString into a [BytesMut].
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    /// use bytes::BytesMut;
    ///
    /// let s = BytesString::from("hello");
    ///
    /// let bytes = s.into_bytes();
    ///
    /// assert_eq!(bytes, BytesMut::from(&b"hello"[..]));
    /// ```
    pub fn into_bytes(self) -> BytesMut {
        self.bytes
    }

    /// Converts a [BytesMut] into a [BytesString] without checking if the bytes
    /// are valid UTF-8.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it does not check if the bytes are valid
    /// UTF-8.
    pub unsafe fn from_bytes_unchecked(bytes: BytesMut) -> Self {
        Self { bytes }
    }

    /// Converts a [BytesMut] into a [BytesString] if the bytes are valid UTF-8.
    ///
    /// # Errors
    ///
    /// Returns a [Utf8Error] if the bytes are not valid UTF-8.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    /// use bytes::BytesMut;
    ///
    /// let s = BytesString::from_utf8(BytesMut::from(&b"hello"[..]));
    /// ```
    pub fn from_utf8(bytes: BytesMut) -> Result<Self, Utf8Error> {
        std::str::from_utf8(bytes.as_ref())?;

        Ok(Self { bytes })
    }

    /// Converts a slice of bytes into a [BytesString] if the bytes are valid
    /// UTF-8.
    ///
    /// # Errors
    ///
    /// Returns a [Utf8Error] if the bytes are not valid UTF-8.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::BytesString;
    ///
    /// let s = BytesString::from_utf8_slice(b"hello");
    /// ```
    pub fn from_utf8_slice(bytes: &[u8]) -> Result<Self, Utf8Error> {
        std::str::from_utf8(bytes)?;

        Ok(Self {
            bytes: BytesMut::from(bytes),
        })
    }
}

impl Deref for BytesString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl DerefMut for BytesString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut_str()
    }
}

impl AsRef<str> for BytesString {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Borrow<str> for BytesString {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<String> for BytesString {
    fn from(s: String) -> Self {
        Self {
            bytes: Bytes::from(s.into_bytes()).into(),
        }
    }
}

impl From<&str> for BytesString {
    fn from(s: &str) -> Self {
        Self {
            bytes: BytesMut::from(s),
        }
    }
}

impl From<BytesString> for BytesMut {
    fn from(s: BytesString) -> Self {
        s.bytes
    }
}

impl From<BytesString> for Bytes {
    fn from(s: BytesString) -> Self {
        s.bytes.into()
    }
}

impl PartialEq<str> for BytesString {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&'_ str> for BytesString {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<BytesString> for str {
    fn eq(&self, other: &BytesString) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<BytesString> for &'_ str {
    fn eq(&self, other: &BytesString) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<BytesString> for Bytes {
    fn eq(&self, other: &BytesString) -> bool {
        self == other.as_bytes()
    }
}

impl PartialEq<String> for BytesString {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<BytesString> for String {
    fn eq(&self, other: &BytesString) -> bool {
        self == other.as_str()
    }
}
