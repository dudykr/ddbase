use std::{
    borrow::Borrow,
    ops::{Deref, DerefMut},
};

use bytes::{Bytes, BytesMut};

/// [String] but backed by a [BytesMut]
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BytesString {
    pub(crate) bytes: BytesMut,
}

impl BytesString {
    /// Returns a new, empty ByteString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let s = ByteString::new();
    ///
    /// assert!(s.is_empty());
    /// ```
    pub fn new() -> Self {
        Self {
            bytes: BytesMut::new(),
        }
    }

    /// Returns a new, empty ByteString with the specified capacity.
    ///
    /// The capacity is the size of the internal buffer in bytes.
    ///
    /// The actual capacity may be larger than the specified capacity.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let s = ByteString::with_capacity(10);
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
    /// use bytes_str::ByteString;
    ///
    /// let s = ByteString::from("hello");
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
    /// use bytes_str::ByteString;
    ///
    /// let s = ByteString::from("hello");
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
    /// use bytes_str::ByteString;
    ///
    /// let mut s = ByteString::from("hello");
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
    /// use bytes_str::ByteString;
    ///
    /// let mut s = ByteString::from("hello");
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
    /// use bytes_str::ByteString;
    ///
    /// let s = ByteString::from("hello");
    ///
    /// assert_eq!(s.as_bytes(), b"hello");
    /// ```
    pub fn as_bytes(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Returns true if the ByteString has a length of 0.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let s = ByteString::new();
    ///
    /// assert!(s.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Truncates the ByteString to the specified length.
    ///
    /// If new_len is greater than or equal to the string’s current length, this
    /// has no effect.
    ///
    /// Note that this method has no effect on the allocated capacity of the
    /// string
    ///
    /// # Arguments
    ///
    /// * `new_len` - The new length of the ByteString
    ///
    /// # Panics
    ///
    /// Panics if new_len does not lie on a char boundary.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let mut s = ByteString::from("hello");
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

    /// Clears the ByteString, removing all bytes.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let mut s = ByteString::from("hello");
    ///
    /// s.clear();
    ///
    /// assert!(s.is_empty());
    /// ```
    pub fn clear(&mut self) {
        self.bytes.clear();
    }

    /// Appends a character to the end of this ByteString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let mut s = ByteString::from("hello");
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

    /// Appends a string slice to the end of this ByteString.
    ///
    /// # Examples
    ///
    /// ```
    /// use bytes_str::ByteString;
    ///
    /// let mut s = ByteString::from("hello");
    ///
    /// s.push_str(" world");
    ///
    /// assert_eq!(s, "hello world");
    /// ```
    pub fn push_str(&mut self, s: &str) {
        self.bytes.extend_from_slice(s.as_bytes());
    }
}

impl Deref for BytesString {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        unsafe { std::str::from_utf8_unchecked(&self.bytes) }
    }
}

impl DerefMut for BytesString {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { std::str::from_utf8_unchecked_mut(&mut self.bytes) }
    }
}

impl AsRef<str> for BytesString {
    fn as_ref(&self) -> &str {
        self.deref()
    }
}

impl Borrow<str> for BytesString {
    fn borrow(&self) -> &str {
        self.as_ref()
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
