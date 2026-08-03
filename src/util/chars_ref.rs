//! A reference to a slice of characters, ported from `org.apache.lucene.util.CharsRef`.
//!
//! Unlike Java's shallow clone, the Rust implementation owns its buffer;
//! [`Clone`] copies the referenced characters so the resulting value is
//! independent and safe.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::fmt::{self, Debug, Display, Formatter};
use std::hash::{Hash, Hasher};

use crate::error::LuceneError;

/// An empty character buffer, equivalent to `CharsRef.EMPTY_CHARS`.
pub const EMPTY_CHARS: &[char] = &[];

/// A reference to a slice of characters, equivalent to Lucene's `CharsRef`.
///
/// Represents a `Vec<char>` together with an `offset` and `length` identifying
/// the active slice. The public fields mirror the Java original.
#[derive(Clone, Default)]
pub struct CharsRef {
    /// The underlying character buffer. An empty reference stores an empty vector.
    pub chars: Vec<char>,
    /// Offset of the first valid character.
    pub offset: usize,
    /// Number of valid characters starting at `offset`.
    pub length: usize,
}

impl CharsRef {
    /// Creates a new empty `CharsRef`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a `CharsRef` whose buffer has the requested capacity and zero length.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            chars: Vec::with_capacity(capacity),
            offset: 0,
            length: 0,
        }
    }

    /// Creates a `CharsRef` referencing the entire provided character vector.
    pub fn from_chars(chars: Vec<char>) -> Self {
        let length = chars.len();
        Self {
            chars,
            offset: 0,
            length,
        }
    }

    /// Creates a `CharsRef` referencing `length` characters of `chars` starting at `offset`.
    ///
    /// # Panics
    ///
    /// Panics if `offset + length > chars.len()`.
    pub fn from_chars_offset(chars: Vec<char>, offset: usize, length: usize) -> Self {
        assert!(
            offset + length <= chars.len(),
            "offset + length out of bounds: offset={offset}, length={length}, chars.len()={}",
            chars.len()
        );
        Self {
            chars,
            offset,
            length,
        }
    }

    /// Creates a `CharsRef` from the characters of `string`.
    pub fn from_string(string: impl AsRef<str>) -> Self {
        let chars: Vec<char> = string.as_ref().chars().collect();
        let length = chars.len();
        Self {
            chars,
            offset: 0,
            length,
        }
    }

    /// Returns the active character slice.
    pub fn chars(&self) -> &[char] {
        &self.chars[self.offset..self.offset + self.length]
    }

    /// Returns the length of the active slice.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Returns the offset of the active slice.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Returns the character at `index` within the active slice.
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.length()`.
    pub fn char_at(&self, index: usize) -> char {
        assert!(
            index < self.length,
            "index {index} out of bounds (length {})",
            self.length
        );
        self.chars[self.offset + index]
    }

    /// Returns a new `CharsRef` containing a copy of the active characters
    /// between `start` and `end` (relative to the active slice), with offset zero.
    ///
    /// # Panics
    ///
    /// Panics if `start > end` or `end > self.length()`.
    pub fn sub_sequence(&self, start: usize, end: usize) -> Self {
        assert!(
            start <= end && end <= self.length,
            "invalid sub-sequence: start={start}, end={end}, length={}",
            self.length
        );
        let slice = &self.chars[self.offset + start..self.offset + end];
        Self {
            chars: slice.to_vec(),
            offset: 0,
            length: end - start,
        }
    }

    /// Returns a deep copy: a `CharsRef` whose buffer contains a copy of the
    /// active characters and whose offset is zero.
    pub fn deep_copy_of(other: &Self) -> Self {
        let slice = other.chars();
        Self {
            chars: slice.to_vec(),
            offset: 0,
            length: slice.len(),
        }
    }

    /// Returns `true` when the active slice equals that of `other`.
    pub fn chars_equals(&self, other: &Self) -> bool {
        self.chars() == other.chars()
    }

    /// Returns the Java `String.hashCode()` of the active character slice.
    pub fn hash_code(&self) -> i32 {
        CharsRef::string_hash_code(self.chars(), 0, self.length)
    }

    /// Returns the Java `String.hashCode()` of `length` characters in `chars`
    /// starting at `offset`.
    ///
    /// The calculation follows `String.hashCode()` exactly, including signed
    /// 32-bit overflow.
    pub fn string_hash_code(chars: &[char], offset: usize, length: usize) -> i32 {
        let mut result = 0i32;
        for &c in chars.iter().skip(offset).take(length) {
            result = result.wrapping_mul(31).wrapping_add(c as i32);
        }
        result
    }

    /// Performs internal consistency checks.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if the offset/length invariants are violated.
    pub fn is_valid(&self) -> Result<(), LuceneError> {
        if self.length > self.chars.len() {
            return Err(LuceneError::IllegalState(format!(
                "length {} out of bounds (chars.len() = {})",
                self.length,
                self.chars.len()
            )));
        }
        if self.offset > self.chars.len() {
            return Err(LuceneError::IllegalState(format!(
                "offset {} out of bounds (chars.len() = {})",
                self.offset,
                self.chars.len()
            )));
        }
        if self.offset + self.length > self.chars.len() {
            return Err(LuceneError::IllegalState(format!(
                "offset + length out of bounds: offset={}, length={}, chars.len()={}",
                self.offset,
                self.length,
                self.chars.len()
            )));
        }
        Ok(())
    }
}

impl From<&str> for CharsRef {
    fn from(value: &str) -> Self {
        Self::from_string(value)
    }
}

impl From<String> for CharsRef {
    fn from(value: String) -> Self {
        Self::from_string(value)
    }
}

impl From<&[char]> for CharsRef {
    fn from(value: &[char]) -> Self {
        Self::from_chars(value.to_vec())
    }
}

impl PartialEq for CharsRef {
    fn eq(&self, other: &Self) -> bool {
        self.chars_equals(other)
    }
}

impl Eq for CharsRef {}

impl PartialOrd for CharsRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for CharsRef {
    fn cmp(&self, other: &Self) -> Ordering {
        self.chars().cmp(other.chars())
    }
}

impl Display for CharsRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for c in self.chars() {
            write!(f, "{c}")?;
        }
        Ok(())
    }
}

impl Debug for CharsRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("CharsRef")
            .field("offset", &self.offset)
            .field("length", &self.length)
            .field("chars", &self.to_string())
            .finish()
    }
}

impl Hash for CharsRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_i32(self.hash_code());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_and_capacity() {
        let empty = CharsRef::new();
        assert_eq!(empty.length(), 0);
        assert_eq!(empty.offset(), 0);
        assert!(empty.chars().is_empty());
        assert!(empty.is_valid().is_ok());

        let with_cap = CharsRef::with_capacity(10);
        assert_eq!(with_cap.length(), 0);
        assert!(with_cap.chars.capacity() >= 10);
    }

    #[test]
    fn from_string_and_slice() {
        let r = CharsRef::from_string("lucene");
        assert_eq!(r.length(), 6);
        assert_eq!(r.to_string(), "lucene");
        assert_eq!(r.chars(), &['l', 'u', 'c', 'e', 'n', 'e']);

        let chars: Vec<char> = "hello world".chars().collect();
        let s = CharsRef::from_chars_offset(chars, 6, 5);
        assert_eq!(s.to_string(), "world");
        assert_eq!(s.char_at(1), 'o');
    }

    #[test]
    fn equality_and_ordering() {
        let a = CharsRef::from_string("abc");
        let b = CharsRef::from_string("abc");
        let c = CharsRef::from_string("abd");
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert!(a < c);
    }

    #[test]
    fn hash_code_matches_java_string_hash() {
        // Java: "lucene".hashCode() wraps around to -1091917150.
        let r = CharsRef::from_string("lucene");
        assert_eq!(r.hash_code(), -1_091_917_150);
    }

    #[test]
    fn deep_copy_and_sub_sequence() {
        let r = CharsRef::from_string("lucene");
        let copy = CharsRef::deep_copy_of(&r);
        assert_eq!(copy, r);
        assert_eq!(copy.offset(), 0);

        let sub = r.sub_sequence(1, 4);
        assert_eq!(sub.to_string(), "uce");
        assert_eq!(sub.offset(), 0);
    }

    #[test]
    fn from_conversions() {
        let from_str: CharsRef = "rust".into();
        assert_eq!(from_str.to_string(), "rust");

        let from_string: CharsRef = String::from("rust").into();
        assert_eq!(from_string.to_string(), "rust");

        let arr = ['r', 'u', 's', 't'];
        let from_arr: CharsRef = (&arr[..]).into();
        assert_eq!(from_arr.to_string(), "rust");
    }
}
