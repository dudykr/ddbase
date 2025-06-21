use std::borrow::Cow;

use ascii::AsciiChar;

// Helper functions for common operations
fn remove_ascii_from_str(s: &str, ch: AsciiChar) -> Option<String> {
    let target_byte = ch.as_byte();
    let bytes = s.as_bytes();

    // Check if the character exists first
    if !bytes.contains(&target_byte) {
        return None;
    }

    // Create new string without the target character
    let mut result = String::with_capacity(s.len());
    for &byte in bytes {
        if byte != target_byte {
            result.push(byte as char);
        }
    }

    Some(result)
}

fn replace_str_if_contains(s: &str, from: &str, to: &str) -> Option<String> {
    if from.is_empty() || !s.contains(from) {
        return None;
    }

    Some(s.replace(from, to))
}

pub trait ReplaceString {
    fn remove_all_ascii(&self, ch: AsciiChar) -> Cow<'_, str>;

    fn remove_all_ascii_in_place(&mut self, ch: AsciiChar);

    fn replace_all_ascii_in_place(&mut self, from: AsciiChar, to: AsciiChar);

    fn replace_all_str(&self, from: &str, to: &str) -> Cow<'_, str>;
}

impl ReplaceString for String {
    fn remove_all_ascii(&self, ch: AsciiChar) -> Cow<'_, str> {
        match remove_ascii_from_str(self, ch) {
            Some(result) => Cow::Owned(result),
            None => Cow::Borrowed(self),
        }
    }

    fn remove_all_ascii_in_place(&mut self, ch: AsciiChar) {
        let target_byte = ch.as_byte();
        let bytes = unsafe { self.as_bytes_mut() };

        let mut write_pos = 0;
        let mut read_pos = 0;

        while read_pos < bytes.len() {
            if bytes[read_pos] != target_byte {
                bytes[write_pos] = bytes[read_pos];
                write_pos += 1;
            }
            read_pos += 1;
        }

        // Truncate to the new length
        self.truncate(write_pos);
    }

    fn replace_all_ascii_in_place(&mut self, from: AsciiChar, to: AsciiChar) {
        let from_byte = from.as_byte();
        let to_byte = to.as_byte();
        let bytes = unsafe { self.as_bytes_mut() };

        for byte in bytes {
            if *byte == from_byte {
                *byte = to_byte;
            }
        }
    }

    fn replace_all_str(&self, from: &str, to: &str) -> Cow<'_, str> {
        match replace_str_if_contains(self, from, to) {
            Some(result) => Cow::Owned(result),
            None => Cow::Borrowed(self),
        }
    }
}

impl ReplaceString for Cow<'_, str> {
    fn remove_all_ascii(&self, ch: AsciiChar) -> Cow<'_, str> {
        match remove_ascii_from_str(self, ch) {
            Some(result) => Cow::Owned(result),
            None => Cow::Borrowed(self),
        }
    }

    fn remove_all_ascii_in_place(&mut self, ch: AsciiChar) {
        match self {
            Cow::Borrowed(s) => {
                if let Some(result) = remove_ascii_from_str(s, ch) {
                    *self = Cow::Owned(result);
                }
            }
            Cow::Owned(s) => {
                s.remove_all_ascii_in_place(ch);
            }
        }
    }

    fn replace_all_ascii_in_place(&mut self, from: AsciiChar, to: AsciiChar) {
        match self {
            Cow::Borrowed(s) => {
                let from_byte = from.as_byte();
                let bytes = s.as_bytes();

                if !bytes.contains(&from_byte) {
                    return; // No changes needed
                }

                // Convert to owned and replace
                let mut owned = s.to_string();
                owned.replace_all_ascii_in_place(from, to);
                *self = Cow::Owned(owned);
            }
            Cow::Owned(s) => {
                s.replace_all_ascii_in_place(from, to);
            }
        }
    }

    fn replace_all_str(&self, from: &str, to: &str) -> Cow<'_, str> {
        match replace_str_if_contains(self, from, to) {
            Some(result) => Cow::Owned(result),
            None => Cow::Borrowed(self),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;

    #[test]
    fn test_string_remove_all_ascii() {
        let s = "hello world".to_string();
        let result = s.remove_all_ascii(AsciiChar::l);
        assert_eq!(result, "heo word");

        // Test with no occurrences
        let s = "hello world".to_string();
        let result = s.remove_all_ascii(AsciiChar::z);
        assert_eq!(result, "hello world");
        match result {
            Cow::Borrowed(_) => {}
            Cow::Owned(_) => panic!("Should return borrowed when no changes"),
        }
    }

    #[test]
    fn test_string_remove_all_ascii_in_place() {
        let mut s = "hello world".to_string();
        s.remove_all_ascii_in_place(AsciiChar::l);
        assert_eq!(s, "heo word");

        let mut s = "aaaaaa".to_string();
        s.remove_all_ascii_in_place(AsciiChar::a);
        assert_eq!(s, "");
    }

    #[test]
    fn test_string_replace_all_ascii_in_place() {
        let mut s = "hello world".to_string();
        s.replace_all_ascii_in_place(AsciiChar::l, AsciiChar::x);
        assert_eq!(s, "hexxo worxd");

        let mut s = "hello world".to_string();
        s.replace_all_ascii_in_place(AsciiChar::z, AsciiChar::x);
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_string_replace_all_str() {
        let s = "hello world hello".to_string();
        let result = s.replace_all_str("hello", "hi");
        assert_eq!(result, "hi world hi");
        assert_eq!(s, "hello world hello"); // Original string should remain unchanged

        // Test with no occurrences
        let s = "hello world".to_string();
        let result = s.replace_all_str("xyz", "abc");
        match result {
            Cow::Borrowed(_) => {}
            Cow::Owned(_) => panic!("Should return borrowed when no changes"),
        }
        assert_eq!(result, "hello world");
        assert_eq!(s, "hello world");
    }

    #[test]
    fn test_cow_remove_all_ascii() {
        let s: Cow<'_, str> = Cow::Borrowed("hello world");
        let result = s.remove_all_ascii(AsciiChar::l);
        assert_eq!(result, "heo word");

        let s: Cow<'_, str> = Cow::Owned("hello world".to_string());
        let result = s.remove_all_ascii(AsciiChar::l);
        assert_eq!(result, "heo word");
    }

    #[test]
    fn test_cow_remove_all_ascii_in_place() {
        let mut s: Cow<'_, str> = Cow::Borrowed("hello world");
        s.remove_all_ascii_in_place(AsciiChar::l);
        assert_eq!(s, "heo word");
        match s {
            Cow::Owned(_) => {}
            Cow::Borrowed(_) => panic!("Should be owned after modification"),
        }

        let mut s: Cow<'_, str> = Cow::Owned("hello world".to_string());
        s.remove_all_ascii_in_place(AsciiChar::l);
        assert_eq!(s, "heo word");
    }

    #[test]
    fn test_cow_replace_all_ascii_in_place() {
        let mut s: Cow<'_, str> = Cow::Borrowed("hello world");
        s.replace_all_ascii_in_place(AsciiChar::l, AsciiChar::x);
        assert_eq!(s, "hexxo worxd");
        match s {
            Cow::Owned(_) => {}
            Cow::Borrowed(_) => panic!("Should be owned after modification"),
        }
    }

    #[test]
    fn test_cow_replace_all_str() {
        let s: Cow<'_, str> = Cow::Borrowed("hello world hello");
        let result = s.replace_all_str("hello", "hi");
        assert_eq!(result, "hi world hi");
        assert_eq!(s, "hello world hello"); // Original string should remain unchanged

        let s: Cow<'_, str> = Cow::Borrowed("hello world");
        let result = s.replace_all_str("xyz", "abc");
        match result {
            Cow::Borrowed(_) => {}
            Cow::Owned(_) => panic!("Should return borrowed when no changes"),
        }
        assert_eq!(result, "hello world");
        assert_eq!(s, "hello world");
    }
}
