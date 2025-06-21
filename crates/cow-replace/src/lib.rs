use std::{borrow::Cow, mem::take};

use ascii::AsciiChar;

pub trait ReplaceString {
    fn remove_all_ascii(&self, ch: AsciiChar) -> Cow<'_, str>;

    fn remove_all_ascii_in_place(&mut self, ch: AsciiChar);

    fn replace_all_ascii_in_place(&mut self, from: AsciiChar, to: AsciiChar);

    fn replace_all_str(&mut self, from: &str, to: &str) -> Cow<'_, str>;
}

impl ReplaceString for String {}

impl ReplaceString for Cow<'_, str> {}
