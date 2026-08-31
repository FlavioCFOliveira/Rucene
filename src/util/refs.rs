//! Reference types and builders ported from `org.apache.lucene.util`.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`LongsRef`] | `LongsRef` |
//! | [`CharsRefBuilder`] | `CharsRefBuilder` |
//! | [`IntsRefBuilder`] | `IntsRefBuilder` |
//!
//! `BytesRef`, `BytesRefBuilder`, `CharsRef` and `IntsRef` are already ported
//! in [`crate::util`], [`crate::util::chars_ref`] and
//! [`crate::util::string_helper`]; the two builders here complete the family.

#![deny(unsafe_code)]

use std::cmp::Ordering;
use std::fmt::{self, Display, Formatter};

use crate::error::{LuceneError, Result};
use crate::util::chars_ref::CharsRef;
use crate::util::string_helper::IntsRef;
use crate::util::unicode_util::UnicodeUtil;
use crate::util::{ArrayUtil, BytesRef};

/// An empty `i64` slice, for convenience. `LongsRef.EMPTY_LONGS`.
pub const EMPTY_LONGS: &[i64] = &[];

// ---------------------------------------------------------------------------
// LongsRef
// ---------------------------------------------------------------------------

/// A slice (offset plus length) into an existing `i64` buffer.
///
/// Port of `org.apache.lucene.util.LongsRef`.
///
/// **Divergence from Lucene 10.5.0.** Java's `LongsRef` is a view over a shared
/// `long[]` and `clone()` is shallow. Rucene's reference types own their
/// buffer, matching [`crate::util::BytesRef`] and
/// [`crate::util::chars_ref::CharsRef`], so `clone()` copies. The content
/// semantics — equality, ordering, hashing, rendering — are unchanged.
#[derive(Clone, Default, Debug)]
pub struct LongsRef {
    /// The contents of this reference.
    pub longs: Vec<i64>,
    /// Offset of the first valid `i64`.
    pub offset: usize,
    /// Number of valid `i64`s starting at `offset`.
    pub length: usize,
}

impl LongsRef {
    /// Creates an empty `LongsRef`. Equivalent to `new LongsRef()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a `LongsRef` backed by a new buffer of `capacity` elements, with
    /// zero offset and length. Equivalent to `new LongsRef(int)`.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            longs: vec![0; capacity],
            offset: 0,
            length: 0,
        }
    }

    /// Creates a `LongsRef` over `longs[offset..offset + length]`.
    ///
    /// Equivalent to `new LongsRef(long[], int, int)`.
    ///
    /// # Panics
    ///
    /// Panics when the resulting reference is not internally consistent, which
    /// is Java's assertion on `isValid()`.
    pub fn from_longs(longs: Vec<i64>, offset: usize, length: usize) -> Self {
        let r = Self {
            longs,
            offset,
            length,
        };
        r.is_valid().expect("INVARIANT: LongsRef must be valid");
        r
    }

    /// Returns the active slice.
    pub fn slice(&self) -> &[i64] {
        &self.longs[self.offset..self.offset + self.length]
    }

    /// Returns whether the active slices are equal.
    ///
    /// Equivalent to `LongsRef.longsEquals`.
    pub fn longs_equals(&self, other: &LongsRef) -> bool {
        self.slice() == other.slice()
    }

    /// Returns a `LongsRef` whose content is a copy of `other`'s active slice,
    /// with offset zero. Equivalent to `LongsRef.deepCopyOf`.
    pub fn deep_copy_of(other: &LongsRef) -> Self {
        Self {
            longs: other.slice().to_vec(),
            offset: 0,
            length: other.length,
        }
    }

    /// Java's `hashCode()`, reproduced exactly so that values hash the same way
    /// on both sides of the port.
    pub fn hash_code(&self) -> i32 {
        const PRIME: i32 = 31;
        let mut result: i32 = 0;
        for &v in self.slice() {
            result = PRIME
                .wrapping_mul(result)
                .wrapping_add((v ^ ((v as u64) >> 32) as i64) as i32);
        }
        result
    }

    /// Performs internal consistency checks.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an invariant is violated,
    /// mirroring Java's `IllegalStateException` messages.
    pub fn is_valid(&self) -> Result<()> {
        if self.length > self.longs.len() {
            return Err(LuceneError::IllegalState(format!(
                "length is out of bounds: {},longs.length={}",
                self.length,
                self.longs.len()
            )));
        }
        if self.offset > self.longs.len() {
            return Err(LuceneError::IllegalState(format!(
                "offset out of bounds: {},longs.length={}",
                self.offset,
                self.longs.len()
            )));
        }
        if self.offset + self.length > self.longs.len() {
            return Err(LuceneError::IllegalState(format!(
                "offset+length out of bounds: offset={},length={},longs.length={}",
                self.offset,
                self.length,
                self.longs.len()
            )));
        }
        Ok(())
    }
}

impl PartialEq for LongsRef {
    fn eq(&self, other: &Self) -> bool {
        self.longs_equals(other)
    }
}

impl Eq for LongsRef {}

impl PartialOrd for LongsRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LongsRef {
    /// Signed `i64` order comparison, as `LongsRef.compareTo` specifies.
    fn cmp(&self, other: &Self) -> Ordering {
        self.slice().cmp(other.slice())
    }
}

impl Display for LongsRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[")?;
        for (i, v) in self.slice().iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            write!(f, "{:x}", *v as u64)?;
        }
        write!(f, "]")
    }
}

// ---------------------------------------------------------------------------
// CharsRefBuilder
// ---------------------------------------------------------------------------

/// The string Java appends for a `null` `CharSequence`. `CharsRefBuilder.NULL_STRING`.
const NULL_STRING: &str = "null";

/// A builder for [`CharsRef`] instances.
///
/// Port of `org.apache.lucene.util.CharsRefBuilder`, which implements
/// `Appendable`.
///
/// **Divergence from Lucene 10.5.0.** Rucene's [`CharsRef`] stores Rust `char`
/// values (Unicode scalar values) rather than Java's UTF-16 code units — a
/// choice already made by [`crate::util::chars_ref`]. `copy_utf8_bytes`
/// therefore produces one element per code point, where Java produces one per
/// UTF-16 code unit; the decoded text is the same. Use
/// [`UnicodeUtil::utf8_to_utf16`] directly when UTF-16 units are required.
#[derive(Debug, Default, Clone)]
pub struct CharsRefBuilder {
    r: CharsRef,
}

impl CharsRefBuilder {
    /// Creates an empty builder. Equivalent to `new CharsRefBuilder()`.
    pub fn new() -> Self {
        Self { r: CharsRef::new() }
    }

    /// Returns the characters of this builder.
    pub fn chars(&self) -> &[char] {
        &self.r.chars
    }

    /// Returns the number of characters in this buffer.
    pub fn length(&self) -> usize {
        self.r.length
    }

    /// Sets the length.
    pub fn set_length(&mut self, length: usize) {
        self.r.length = length;
    }

    /// Returns the character at `offset`.
    pub fn char_at(&self, offset: usize) -> char {
        self.r.chars[offset]
    }

    /// Sets the character at `offset`.
    pub fn set_char_at(&mut self, offset: usize, c: char) {
        self.r.chars[offset] = c;
    }

    /// Resets this builder to the empty state.
    pub fn clear(&mut self) {
        self.r.length = 0;
    }

    /// Appends the characters of `csq`, or the literal `null` when `None`.
    ///
    /// Equivalent to `CharsRefBuilder.append(CharSequence)`.
    pub fn append_str(&mut self, csq: Option<&str>) -> &mut Self {
        match csq {
            None => self.append_str(Some(NULL_STRING)),
            Some(s) => {
                let chars: Vec<char> = s.chars().collect();
                self.append_chars(&chars, 0, chars.len());
                self
            }
        }
    }

    /// Appends a single character. Equivalent to `CharsRefBuilder.append(char)`.
    pub fn append(&mut self, c: char) -> &mut Self {
        self.grow(self.r.length + 1);
        let len = self.r.length;
        self.set_char_at(len, c);
        self.r.length += 1;
        self
    }

    /// Copies `other`'s referenced content into this builder.
    ///
    /// Equivalent to `CharsRefBuilder.copyChars(CharsRef)`.
    pub fn copy_chars_ref(&mut self, other: &CharsRef) {
        self.copy_chars(&other.chars, other.offset, other.length);
    }

    /// Grows the reference array to hold at least `new_length` characters.
    pub fn grow(&mut self, new_length: usize) {
        if self.r.chars.len() < new_length {
            let target = ArrayUtil::oversize(new_length, 4).max(new_length);
            self.r.chars.resize(target, '\0');
        }
    }

    /// Copies the provided bytes, interpreted as UTF-8.
    ///
    /// Equivalent to `CharsRefBuilder.copyUTF8Bytes(byte[], int, int)`.
    pub fn copy_utf8_bytes(&mut self, bytes: &[u8], offset: usize, length: usize) {
        self.grow(length);
        let mut units = vec![0u16; length];
        let n = UnicodeUtil::utf8_to_utf16(bytes, offset, length, &mut units);
        let decoded: Vec<char> = char::decode_utf16(units[..n].iter().copied())
            .map(|r| r.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        self.grow(decoded.len());
        self.r.chars[..decoded.len()].copy_from_slice(&decoded);
        self.r.length = decoded.len();
    }

    /// Copies the provided [`BytesRef`], interpreted as UTF-8.
    ///
    /// Equivalent to `CharsRefBuilder.copyUTF8Bytes(BytesRef)`.
    pub fn copy_utf8_bytes_ref(&mut self, bytes: &BytesRef) {
        self.copy_utf8_bytes(&bytes.bytes, bytes.offset, bytes.length);
    }

    /// Copies `other_chars[other_offset..other_offset + other_length]` into this
    /// builder, replacing its content.
    ///
    /// Equivalent to `CharsRefBuilder.copyChars(char[], int, int)`.
    pub fn copy_chars(&mut self, other_chars: &[char], other_offset: usize, other_length: usize) {
        self.grow(other_length);
        self.r.chars[..other_length]
            .copy_from_slice(&other_chars[other_offset..other_offset + other_length]);
        self.r.length = other_length;
    }

    /// Appends `other_chars[other_offset..other_offset + other_length]`.
    ///
    /// Equivalent to `CharsRefBuilder.append(char[], int, int)`.
    pub fn append_chars(&mut self, other_chars: &[char], other_offset: usize, other_length: usize) {
        let new_len = self.r.length + other_length;
        self.grow(new_len);
        let start = self.r.length;
        self.r.chars[start..new_len]
            .copy_from_slice(&other_chars[other_offset..other_offset + other_length]);
        self.r.length = new_len;
    }

    /// Returns a [`CharsRef`] holding the current content.
    ///
    /// Equivalent to `CharsRefBuilder.get()`. Java returns a view over the
    /// builder's live array; Rucene's `CharsRef` owns its buffer, so this is a
    /// snapshot.
    pub fn get(&self) -> CharsRef {
        debug_assert_eq!(
            self.r.offset, 0,
            "Modifying the offset of the returned ref is illegal"
        );
        CharsRef::from_chars_offset(self.r.chars[..self.r.length].to_vec(), 0, self.r.length)
    }

    /// Builds a new [`CharsRef`] with the same content as this builder.
    ///
    /// Equivalent to `CharsRefBuilder.toCharsRef()`.
    pub fn to_chars_ref(&self) -> CharsRef {
        self.get()
    }
}

impl Display for CharsRefBuilder {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get())
    }
}

// ---------------------------------------------------------------------------
// IntsRefBuilder
// ---------------------------------------------------------------------------

/// A builder for [`IntsRef`] instances.
///
/// Port of `org.apache.lucene.util.IntsRefBuilder`.
#[derive(Debug, Default, Clone)]
pub struct IntsRefBuilder {
    r: IntsRef,
}

impl IntsRefBuilder {
    /// Creates an empty builder. Equivalent to `new IntsRefBuilder()`.
    pub fn new() -> Self {
        Self {
            r: IntsRef::default(),
        }
    }

    /// Returns the ints of this builder.
    pub fn ints(&self) -> &[i32] {
        &self.r.ints
    }

    /// Returns a mutable view of the ints of this builder.
    pub fn ints_mut(&mut self) -> &mut [i32] {
        &mut self.r.ints
    }

    /// Returns the number of ints in this buffer.
    pub fn length(&self) -> usize {
        self.r.length
    }

    /// Sets the length.
    pub fn set_length(&mut self, length: usize) {
        self.r.length = length;
    }

    /// Empties this builder.
    pub fn clear(&mut self) {
        self.set_length(0);
    }

    /// Returns the int at `offset`.
    pub fn int_at(&self, offset: usize) -> i32 {
        self.r.ints[offset]
    }

    /// Sets the int at `offset`.
    pub fn set_int_at(&mut self, offset: usize, b: i32) {
        self.r.ints[offset] = b;
    }

    /// Appends `i` to this buffer.
    pub fn append(&mut self, i: i32) {
        self.grow(self.r.length + 1);
        let len = self.r.length;
        self.r.ints[len] = i;
        self.r.length += 1;
    }

    /// Grows the reference array to hold at least `new_length` ints, preserving
    /// the existing content.
    ///
    /// Does not take the offset into account, exactly as
    /// `IntsRefBuilder.grow` documents.
    pub fn grow(&mut self, new_length: usize) {
        if self.r.ints.len() < new_length {
            let target = ArrayUtil::oversize(new_length, 4).max(new_length);
            self.r.ints.resize(target, 0);
        }
    }

    /// Grows the reference array without preserving the existing content.
    ///
    /// Equivalent to `IntsRefBuilder.growNoCopy`.
    pub fn grow_no_copy(&mut self, new_length: usize) {
        if self.r.ints.len() < new_length {
            let target = ArrayUtil::oversize(new_length, 4).max(new_length);
            self.r.ints = vec![0; target];
        }
    }

    /// Copies `other_ints[other_offset..other_offset + other_length]` into this
    /// builder. Equivalent to `IntsRefBuilder.copyInts(int[], int, int)`.
    pub fn copy_ints(&mut self, other_ints: &[i32], other_offset: usize, other_length: usize) {
        self.grow_no_copy(other_length);
        self.r.ints[..other_length]
            .copy_from_slice(&other_ints[other_offset..other_offset + other_length]);
        self.r.length = other_length;
    }

    /// Copies `ints` into this builder.
    ///
    /// Equivalent to `IntsRefBuilder.copyInts(IntsRef)`.
    pub fn copy_ints_ref(&mut self, ints: &IntsRef) {
        self.copy_ints(&ints.ints, ints.offset, ints.length);
    }

    /// Copies the given UTF-8 bytes into this builder as UTF-32 code points.
    ///
    /// Equivalent to `IntsRefBuilder.copyUTF8Bytes(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the bytes are not valid
    /// UTF-8.
    pub fn copy_utf8_bytes(&mut self, bytes: &BytesRef) -> Result<()> {
        self.grow_no_copy(bytes.length);
        let n = {
            let Self { r } = self;
            UnicodeUtil::utf8_to_utf32(bytes, &mut r.ints)?
        };
        self.r.length = n;
        Ok(())
    }

    /// Returns an [`IntsRef`] holding the current content.
    ///
    /// Equivalent to `IntsRefBuilder.get()`; as with [`CharsRefBuilder::get`],
    /// Rucene's owning reference types make this a snapshot rather than a view.
    pub fn get(&self) -> IntsRef {
        debug_assert_eq!(
            self.r.offset, 0,
            "Modifying the offset of the returned ref is illegal"
        );
        IntsRef {
            ints: self.r.ints[..self.r.length].to_vec(),
            offset: 0,
            length: self.r.length,
        }
    }

    /// Builds a new [`IntsRef`] with the same content as this builder.
    ///
    /// Equivalent to `IntsRefBuilder.toIntsRef()`.
    pub fn to_ints_ref(&self) -> IntsRef {
        self.get()
    }
}
