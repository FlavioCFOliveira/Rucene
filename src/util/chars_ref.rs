//! A reference to a slice of characters, ported from `org.apache.lucene.util.CharsRef`.
//!
//! Characters are UTF-16 code units (`u16`), as Java's `char[]`.
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
pub const EMPTY_CHARS: &[u16] = &[];

/// A reference to a slice of characters, equivalent to Lucene's `CharsRef`.
///
/// Represents a `Vec<u16>` of UTF-16 code units (Java's `char[]`) together with an `offset` and `length` identifying
/// the active slice. The public fields mirror the Java original.
#[derive(Clone, Default)]
pub struct CharsRef {
    /// The underlying character buffer. An empty reference stores an empty vector.
    pub chars: Vec<u16>,
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

    /// Creates a `CharsRef` over a zero-filled buffer of `capacity` characters,
    /// with zero length.
    ///
    /// Equivalent to `new CharsRef(int capacity)` (`CharsRef.java:47-49`),
    /// whose `chars = new char[capacity]` holds `capacity` NUL characters.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            chars: vec![0u16; capacity],
            offset: 0,
            length: 0,
        }
    }

    /// Creates a `CharsRef` referencing the entire provided character vector.
    pub fn from_chars(chars: Vec<u16>) -> Self {
        let length = chars.len();
        Self {
            chars,
            offset: 0,
            length,
        }
    }

    /// Creates a `CharsRef` referencing `length` characters of `chars` starting at `offset`.
    ///
    /// Equivalent to `new CharsRef(char[], int, int)` (`CharsRef.java:52-57`),
    /// which ends with `assert isValid()`.
    ///
    /// # Panics
    ///
    /// Panics in builds with debug assertions when [`is_valid`](Self::is_valid)
    /// fails, as Java's assertion does under `-ea`.
    pub fn from_chars_offset(chars: Vec<u16>, offset: usize, length: usize) -> Self {
        let r = Self {
            chars,
            offset,
            length,
        };
        if let Err(e) = r.is_valid() {
            debug_assert!(false, "{e}");
        }
        r
    }

    /// Creates a `CharsRef` from the UTF-16 code units of `string`.
    ///
    /// Equivalent to `new CharsRef(String)` (`CharsRef.java:61-65`), whose
    /// `string.toCharArray()` yields UTF-16 code units.
    pub fn from_string(string: impl AsRef<str>) -> Self {
        let chars: Vec<u16> = string.as_ref().encode_utf16().collect();
        let length = chars.len();
        Self {
            chars,
            offset: 0,
            length,
        }
    }

    /// Returns the active character slice.
    pub fn chars(&self) -> &[u16] {
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
    /// Equivalent to `CharsRef.charAt(int)` (`CharsRef.java:139-144`).
    ///
    /// # Panics
    ///
    /// Panics if `index >= self.length()`, with the message of the
    /// `IndexOutOfBoundsException` Java's `Objects.checkIndex(index, length)`
    /// throws: `Index <index> out of bounds for length <length>`.
    pub fn char_at(&self, index: usize) -> u16 {
        assert!(
            index < self.length,
            "Index {index} out of bounds for length {}",
            self.length
        );
        self.chars[self.offset + index]
    }

    /// Returns a new `CharsRef` containing a copy of the active characters
    /// between `start` and `end` (relative to the active slice), with offset zero.
    ///
    /// Equivalent to `CharsRef.subSequence(int, int)` (`CharsRef.java:146-150`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns
    /// `new CharsRef(chars, offset + start, end - start)`, sharing this array;
    /// a `Vec<u16>` has one owner, so the characters are copied and the
    /// result's offset is zero. The characters it denotes are the same.
    ///
    /// # Panics
    ///
    /// Panics if `start > end` or `end > self.length()`, with the message of
    /// the `IndexOutOfBoundsException` Java's `Objects.checkFromToIndex(start,
    /// end, length)` throws: `Range [<start>, <end>) out of bounds for length
    /// <length>`.
    pub fn sub_sequence(&self, start: usize, end: usize) -> Self {
        assert!(
            start <= end && end <= self.length,
            "Range [{start}, {end}) out of bounds for length {}",
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
    pub fn string_hash_code(chars: &[u16], offset: usize, length: usize) -> i32 {
        let mut result = 0i32;
        for &c in chars.iter().skip(offset).take(length) {
            result = result.wrapping_mul(31).wrapping_add(c as i32);
        }
        result
    }

    /// Returns the comparator that orders UTF-16 sequences by the order their
    /// UTF-8 encodings would have.
    ///
    /// Equivalent to `CharsRef.getUTF16SortedAsUTF8Comparator()`, which Java
    /// marks deprecated as a transition mechanism and keeps for the callers
    /// that still need it. Java returns a singleton `Comparator`; a function
    /// pointer is the same thing here.
    pub fn get_utf16_sorted_as_utf8_comparator() -> fn(&Self, &Self) -> std::cmp::Ordering {
        Self::utf16_sorted_as_utf8_compare
    }

    /// Orders two sequences by the order their UTF-8 encodings would have.
    ///
    /// Equivalent to `CharsRef.UTF16SortedAsUTF8Comparator.compare(CharsRef,
    /// CharsRef)`. The surrogate fix-up is the one described in
    /// <http://icu-project.org/docs/papers/utf16_code_point_order.html>.
    fn utf16_sorted_as_utf8_compare(a: &Self, b: &Self) -> std::cmp::Ordering {
        let a_chars = &a.chars[a.offset..a.offset + a.length];
        let b_chars = &b.chars[b.offset..b.offset + b.length];
        let mismatch = a_chars.iter().zip(b_chars.iter()).position(|(x, y)| x != y);

        if let Some(i) = mismatch {
            let mut a_char = u32::from(a_chars[i]);
            let mut b_char = u32::from(b_chars[i]);
            // aChar != bChar; fix up each one if they are both in or above the
            // surrogate range, then compare them.
            if a_char >= 0xd800 && b_char >= 0xd800 {
                if a_char >= 0xe000 {
                    a_char -= 0x800;
                } else {
                    a_char += 0x2000;
                }
                if b_char >= 0xe000 {
                    b_char -= 0x800;
                } else {
                    b_char += 0x2000;
                }
            }
            // Now a_char and b_char are in code-point order.
            return a_char.cmp(&b_char);
        }

        // One is a prefix of the other, or they are equal.
        a.length.cmp(&b.length)
    }

    /// Performs internal consistency checks.
    ///
    /// Equivalent to `CharsRef.isValid()`, which always returns `true` or throws
    /// `IllegalStateException`; this returns `Ok(())` or the matching error.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when a check fails.
    pub fn is_valid(&self) -> Result<(), LuceneError> {
        // `chars` cannot be null, and the `usize` fields cannot be negative, so
        // Java's four checks for those have no Rust counterpart.
        if self.length > self.chars.len() {
            return Err(LuceneError::illegal_state(format!(
                "length is out of bounds: {},chars.length={}",
                self.length,
                self.chars.len()
            )));
        }
        if self.offset > self.chars.len() {
            return Err(LuceneError::illegal_state(format!(
                "offset out of bounds: {},chars.length={}",
                self.offset,
                self.chars.len()
            )));
        }
        if self.offset + self.length > self.chars.len() {
            return Err(LuceneError::illegal_state(format!(
                "offset+length out of bounds: offset={},length={},chars.length={}",
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

impl From<&[u16]> for CharsRef {
    fn from(value: &[u16]) -> Self {
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

/// Equivalent to `CharsRef.toString()` (`new String(chars, offset, length)`).
///
/// **Divergence.** A Java `String` keeps an unpaired surrogate; a Rust
/// `String` cannot hold one, so each is written as U+FFFD.
impl Display for CharsRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        for c in char::decode_utf16(self.chars().iter().copied()) {
            write!(f, "{}", c.unwrap_or(char::REPLACEMENT_CHARACTER))?;
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
    /// `Objects.checkIndex` and `Objects.checkFromToIndex` messages. Measured
    /// against `lucene-core-10.5.0.jar`'s JDK (`oracle/fix/MiscFix.java`,
    /// item 27): `Index 5 out of bounds for length 3`.
    #[test]
    #[should_panic(expected = "Index 5 out of bounds for length 3")]
    fn char_at_out_of_bounds_has_the_jdk_message() {
        super::CharsRef::from_string("abc").char_at(5);
    }

    /// As above: `Range [2, 5) out of bounds for length 3`.
    #[test]
    #[should_panic(expected = "Range [2, 5) out of bounds for length 3")]
    fn sub_sequence_out_of_bounds_has_the_jdk_message() {
        let _ = super::CharsRef::from_string("abc").sub_sequence(2, 5);
    }

    use super::*;

    #[test]
    fn empty_and_capacity() {
        let empty = CharsRef::new();
        assert_eq!(empty.length(), 0);
        assert_eq!(empty.offset(), 0);
        assert!(empty.chars().is_empty());
        assert!(empty.is_valid().is_ok());

        // `new CharsRef(10)`: `chars = new char[10]`, ten NULs, length 0.
        let with_cap = CharsRef::with_capacity(10);
        assert_eq!(with_cap.length(), 0);
        assert_eq!(with_cap.chars, vec![0u16; 10]);
    }

    /// `new CharsRef(char[], int, int)` ends with `assert isValid()`
    /// (`CharsRef.java:56`), whose message for a range past the end is
    /// `offset+length out of bounds: offset=…,length=…,chars.length=…`.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "offset+length out of bounds: offset=2,length=2,chars.length=3")]
    fn from_chars_offset_asserts_is_valid() {
        let _ = CharsRef::from_chars_offset("abc".encode_utf16().collect(), 2, 2);
    }

    #[test]
    fn is_valid_reports_lucenes_messages() {
        let mut r = CharsRef::from_string("abc");
        r.length = 4;
        assert_eq!(
            r.is_valid().unwrap_err().to_string(),
            LuceneError::illegal_state("length is out of bounds: 4,chars.length=3".to_string())
                .to_string()
        );
        r.length = 0;
        r.offset = 4;
        assert_eq!(
            r.is_valid().unwrap_err().to_string(),
            LuceneError::illegal_state("offset out of bounds: 4,chars.length=3".to_string())
                .to_string()
        );
    }

    #[test]
    fn from_string_and_slice() {
        let r = CharsRef::from_string("lucene");
        assert_eq!(r.length(), 6);
        assert_eq!(r.to_string(), "lucene");
        assert_eq!(r.chars(), "lucene".encode_utf16().collect::<Vec<u16>>());

        let chars: Vec<u16> = "hello world".encode_utf16().collect();
        let s = CharsRef::from_chars_offset(chars, 6, 5);
        assert_eq!(s.to_string(), "world");
        assert_eq!(s.char_at(1), u16::from(b'o'));
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

        let arr: Vec<u16> = "rust".encode_utf16().collect();
        let from_arr: CharsRef = (&arr[..]).into();
        assert_eq!(from_arr.to_string(), "rust");
    }
}
