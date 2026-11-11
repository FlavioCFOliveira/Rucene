//! String and byte-reference helpers ported from `org.apache.lucene.util.StringHelper`.
//!
//! This module also provides standalone [`write_string`] and [`read_string`] helpers
//! that produce/consume the same byte serialization used by Lucene's
//! `DataOutput.writeString` / `DataInput.readString`: a VInt length followed by
//! UTF-8 bytes. These functions are kept inside `util` so they do not introduce a
//! dependency from `util` on the `store` module.

#![deny(unsafe_code)]

use std::{
    cmp::Ordering,
    fmt,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{error::LuceneError, util::BitUtil};

use super::BytesRef;

// ---------------------------------------------------------------------------
// IntsRef
// ---------------------------------------------------------------------------

/// Port of `org.apache.lucene.util.IntsRef`.
///
/// A reference to a slice of `i32` values, equivalent to Lucene's `IntsRef`.
#[derive(Default, Debug)]
pub struct IntsRef {
    /// The underlying integer buffer.
    pub ints: Vec<i32>,
    /// Offset of the first valid integer.
    pub offset: usize,
    /// Number of valid integers starting at `offset`.
    pub length: usize,
}

impl IntsRef {
    /// Creates an `IntsRef` referencing the entire provided vector.
    pub fn new(ints: Vec<i32>) -> Self {
        let length = ints.len();
        Self {
            ints,
            offset: 0,
            length,
        }
    }

    /// References `ints` directly, without copying it, with the given offset
    /// and length.
    ///
    /// Equivalent to `IntsRef(int[], int, int)` (`IntsRef.java:54-59`), which
    /// ends with `assert isValid()`. Java's `isValid()` throws an
    /// `IllegalStateException` describing the broken invariant, and the `assert`
    /// only runs it under `-ea`; this port checks it with `debug_assert!`, so
    /// the check exists in debug builds only, and panics with that message.
    ///
    /// # Panics
    ///
    /// In debug builds, when `offset` or `offset + length` falls outside
    /// `ints`, with the message of the `IllegalStateException` from
    /// [`is_valid`](Self::is_valid).
    pub fn with_offset_length(ints: Vec<i32>, offset: usize, length: usize) -> Self {
        let ints_ref = Self {
            ints,
            offset,
            length,
        };
        debug_assert!(
            ints_ref.is_valid().is_ok(),
            "{}",
            ints_ref
                .is_valid()
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default()
        );
        ints_ref
    }

    /// Returns the active slice.
    pub fn slice(&self) -> &[i32] {
        &self.ints[self.offset..self.offset + self.length]
    }

    /// Returns whether `other` holds the same integers as this reference.
    ///
    /// Equivalent to `IntsRef.intsEquals(IntsRef)`.
    pub fn ints_equals(&self, other: &Self) -> bool {
        self.slice() == other.slice()
    }

    /// Returns an `IntsRef` whose content is a copy of `other`'s active slice,
    /// with offset zero and length `other.length`.
    ///
    /// Equivalent to `org.apache.lucene.util.IntsRef#deepCopyOf`.
    pub fn deep_copy_of(other: &IntsRef) -> Self {
        // `ArrayUtil.copyOfSubArray(other.ints, other.offset, other.offset +
        // other.length)` (`IntsRef.java:136-140`).
        let ints =
            crate::util::java_copy_of_sub_array(&other.ints, other.offset, other.length, "int");
        Self::with_offset_length(ints, 0, other.length)
    }

    /// Java's `hashCode()`, reproduced exactly so that values hash the same way
    /// on both sides of the port.
    ///
    /// Equivalent to `org.apache.lucene.util.IntsRef#hashCode`.
    pub fn hash_code(&self) -> i32 {
        const PRIME: i32 = 31;
        let mut result: i32 = 0;
        for &v in self.slice() {
            result = PRIME.wrapping_mul(result).wrapping_add(v);
        }
        result
    }

    /// Performs internal consistency checks.
    ///
    /// Java's `ints` can be `null` and its `offset`/`length` can be negative;
    /// neither is representable here, so the corresponding checks are dropped
    /// and the remaining messages are reproduced verbatim.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when an invariant is violated,
    /// mirroring Java's `IllegalStateException` messages.
    ///
    /// Equivalent to `org.apache.lucene.util.IntsRef#isValid`.
    pub fn is_valid(&self) -> Result<(), LuceneError> {
        if self.length > self.ints.len() {
            return Err(LuceneError::illegal_state(format!(
                "length is out of bounds: {},ints.length={}",
                self.length,
                self.ints.len()
            )));
        }
        if self.offset > self.ints.len() {
            return Err(LuceneError::illegal_state(format!(
                "offset out of bounds: {},ints.length={}",
                self.offset,
                self.ints.len()
            )));
        }
        if self.offset + self.length > self.ints.len() {
            return Err(LuceneError::illegal_state(format!(
                "offset+length out of bounds: offset={},length={},ints.length={}",
                self.offset,
                self.length,
                self.ints.len()
            )));
        }
        Ok(())
    }
}

impl fmt::Display for IntsRef {
    /// Port of `IntsRef.toString()`: the active slice rendered as unsigned
    /// hexadecimal, one value per `Integer.toHexString` call, separated by a
    /// single space and wrapped in square brackets.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[")?;
        for (i, value) in self.slice().iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            f.write_str(&crate::util::java_fmt::to_hex_string(*value))?;
        }
        f.write_str("]")
    }
}

impl Clone for IntsRef {
    /// Equivalent to `IntsRef.clone()` (`IntsRef.java:68-70`), which returns
    /// `new IntsRef(ints, offset, length)` and so runs that constructor's
    /// `assert isValid()`. Java's clone shares `ints`; this owning port copies
    /// them, keeping the same offset and length.
    fn clone(&self) -> Self {
        Self::with_offset_length(self.ints.clone(), self.offset, self.length)
    }
}

impl PartialEq for IntsRef {
    fn eq(&self, other: &Self) -> bool {
        self.ints_equals(other)
    }
}

impl Eq for IntsRef {}

impl PartialOrd for IntsRef {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for IntsRef {
    /// Signed `i32` order comparison.
    ///
    /// Equivalent to `org.apache.lucene.util.IntsRef#compareTo`.
    fn cmp(&self, other: &Self) -> Ordering {
        self.slice().cmp(other.slice())
    }
}

// ---------------------------------------------------------------------------
// StringHelper
// ---------------------------------------------------------------------------

/// Helpers for manipulating strings and byte references, equivalent to Lucene's
/// `StringHelper`.
pub struct StringHelper;

impl StringHelper {
    /// Compares two [`BytesRef`] values element by element and returns the number
    /// of common elements from the start of each.
    ///
    /// Equivalent to `StringHelper.bytesDifference(BytesRef, BytesRef)`
    /// (`StringHelper.java:42-56`), which is `Arrays.mismatch` over the two
    /// slices: the first differing index, or the shorter length when one is a
    /// prefix of the other, and `-1` only when the slices are equal.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` (`terms out of order: …`)
    /// when the two slices are equal, the only case `Arrays.mismatch` answers
    /// `-1`.
    pub fn bytes_difference(
        prior_term: &BytesRef,
        current_term: &BytesRef,
    ) -> Result<i32, LuceneError> {
        let a = prior_term.slice();
        let b = current_term.slice();
        if a == b {
            return Err(LuceneError::illegal_argument(format!(
                "terms out of order: priorTerm={prior_term},currentTerm={current_term}"
            )));
        }
        let mismatch = a
            .iter()
            .zip(b)
            .position(|(x, y)| x != y)
            .unwrap_or(a.len().min(b.len()));
        mismatch
            .try_into()
            .map_err(|_| LuceneError::illegal_state("mismatch index overflow".to_string()))
    }

    /// Returns the length of `current_term` needed as a sort key so that byte
    /// comparison still produces the same result as the full term comparison.
    ///
    /// This assumes `current_term` comes after `prior_term`.
    pub fn sort_key_length(
        prior_term: &BytesRef,
        current_term: &BytesRef,
    ) -> Result<i32, LuceneError> {
        Ok(Self::bytes_difference(prior_term, current_term)? + 1)
    }

    /// Returns true iff `ref` starts with `prefix`.
    pub fn starts_with_bytes(ref_bytes: &[u8], prefix: &BytesRef) -> bool {
        if ref_bytes.len() < prefix.length {
            return false;
        }
        ref_bytes[..prefix.length] == *prefix.slice()
    }

    /// Returns true iff `ref` starts with `prefix`.
    pub fn starts_with(ref_: &BytesRef, prefix: &BytesRef) -> bool {
        if ref_.length < prefix.length {
            return false;
        }
        ref_.slice()[..prefix.length] == *prefix.slice()
    }

    /// Returns true iff `ref` ends with `suffix`.
    pub fn ends_with(ref_: &BytesRef, suffix: &BytesRef) -> bool {
        let start_at = ref_.length.checked_sub(suffix.length);
        let Some(start_at) = start_at else {
            return false;
        };
        ref_.slice()[start_at..start_at + suffix.length] == *suffix.slice()
    }

    /// Returns the MurmurHash3 x86 32-bit hash of `data[offset..offset+len]`.
    pub fn murmurhash3_x86_32(data: &[u8], offset: i32, len: i32, seed: i32) -> i32 {
        let offset = offset as usize;
        let len = len as usize;
        let c1: i32 = 0xcc9e2d51u32 as i32;
        let c2: i32 = 0x1b873593u32 as i32;

        let mut h1 = seed;
        let rounded_end = offset + (len & 0xfffffffc);

        let mut i = offset;
        while i < rounded_end {
            let k1 = BitUtil::read_le_int(data, i);
            let mut k1 = k1.wrapping_mul(c1);
            k1 = k1.rotate_left(15);
            k1 = k1.wrapping_mul(c2);

            h1 ^= k1;
            h1 = h1.rotate_left(13);
            h1 = h1.wrapping_mul(5).wrapping_add(0xe6546b64u32 as i32);
            i += 4;
        }

        match len & 0x03 {
            3 => {
                let mut k1 = ((data[rounded_end + 2] as i32) & 0xff) << 16;
                k1 |= ((data[rounded_end + 1] as i32) & 0xff) << 8;
                k1 |= (data[rounded_end] as i32) & 0xff;
                let mut k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(15);
                let k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            2 => {
                let mut k1 = ((data[rounded_end + 1] as i32) & 0xff) << 8;
                k1 |= (data[rounded_end] as i32) & 0xff;
                let mut k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(15);
                let k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            1 => {
                let mut k1 = (data[rounded_end] as i32) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(15);
                let k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            _ => {}
        }

        h1 ^= len as i32;

        h1 ^= (h1 as u32 >> 16) as i32;
        h1 = h1.wrapping_mul(0x85ebca6bu32 as i32);
        h1 ^= (h1 as u32 >> 13) as i32;
        h1 = h1.wrapping_mul(0xc2b2ae35u32 as i32);
        h1 ^= (h1 as u32 >> 16) as i32;

        h1
    }

    /// Returns the MurmurHash3 x86 32-bit hash of the active slice of `bytes`.
    pub fn murmurhash3_x86_32_bytes_ref(bytes: &BytesRef, seed: i32) -> i32 {
        Self::murmurhash3_x86_32(&bytes.bytes, bytes.offset as i32, bytes.length as i32, seed)
    }

    /// Returns the 128-bit MurmurHash3 x64 hash of `data[offset..offset+length]`.
    pub fn murmurhash3_x64_128(data: &[u8], offset: i32, length: i32, seed: i32) -> [i64; 2] {
        let offset = offset as usize;
        let length = length as usize;
        let seed = (seed as u32) as i64;

        let c1: i64 = 0x87c37b91114253d5u64 as i64;
        let c2: i64 = 0x4cf5ad432745937fu64 as i64;
        let r1 = 31;
        let r2 = 27;
        let r3 = 33;
        let m = 5;
        let n1: i64 = 0x52dce729u32 as i64;
        let n2: i64 = 0x38495ab5u32 as i64;

        let mut h1 = seed;
        let mut h2 = seed;
        let nblocks = length >> 4;

        for i in 0..nblocks {
            let idx = offset + (i << 4);
            let mut k1 = BitUtil::read_le_long(data, idx);
            let mut k2 = BitUtil::read_le_long(data, idx + 8);

            k1 = k1.wrapping_mul(c1);
            k1 = k1.rotate_left(r1);
            k1 = k1.wrapping_mul(c2);
            h1 ^= k1;
            h1 = h1.rotate_left(r2);
            h1 = h1.wrapping_add(h2);
            h1 = h1.wrapping_mul(m).wrapping_add(n1);

            k2 = k2.wrapping_mul(c2);
            k2 = k2.rotate_left(r3);
            k2 = k2.wrapping_mul(c1);
            h2 ^= k2;
            h2 = h2.rotate_left(r1);
            h2 = h2.wrapping_add(h1);
            h2 = h2.wrapping_mul(m).wrapping_add(n2);
        }

        let mut k1: i64 = 0;
        let mut k2: i64 = 0;
        let idx = offset + (nblocks << 4);
        match length & 0x0f {
            15 => {
                k2 ^= ((data[idx + 14] as i64) & 0xff) << 48;
                k2 ^= ((data[idx + 13] as i64) & 0xff) << 40;
                k2 ^= ((data[idx + 12] as i64) & 0xff) << 32;
                k2 ^= ((data[idx + 11] as i64) & 0xff) << 24;
                k2 ^= ((data[idx + 10] as i64) & 0xff) << 16;
                k2 ^= ((data[idx + 9] as i64) & 0xff) << 8;
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            14 => {
                k2 ^= ((data[idx + 13] as i64) & 0xff) << 40;
                k2 ^= ((data[idx + 12] as i64) & 0xff) << 32;
                k2 ^= ((data[idx + 11] as i64) & 0xff) << 24;
                k2 ^= ((data[idx + 10] as i64) & 0xff) << 16;
                k2 ^= ((data[idx + 9] as i64) & 0xff) << 8;
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            13 => {
                k2 ^= ((data[idx + 12] as i64) & 0xff) << 32;
                k2 ^= ((data[idx + 11] as i64) & 0xff) << 24;
                k2 ^= ((data[idx + 10] as i64) & 0xff) << 16;
                k2 ^= ((data[idx + 9] as i64) & 0xff) << 8;
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            12 => {
                k2 ^= ((data[idx + 11] as i64) & 0xff) << 24;
                k2 ^= ((data[idx + 10] as i64) & 0xff) << 16;
                k2 ^= ((data[idx + 9] as i64) & 0xff) << 8;
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            11 => {
                k2 ^= ((data[idx + 10] as i64) & 0xff) << 16;
                k2 ^= ((data[idx + 9] as i64) & 0xff) << 8;
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            10 => {
                k2 ^= ((data[idx + 9] as i64) & 0xff) << 8;
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            9 => {
                k2 ^= (data[idx + 8] as i64) & 0xff;
                k2 = k2.wrapping_mul(c2);
                k2 = k2.rotate_left(r3);
                k2 = k2.wrapping_mul(c1);
                h2 ^= k2;

                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            8 => {
                k1 ^= ((data[idx + 7] as i64) & 0xff) << 56;
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            7 => {
                k1 ^= ((data[idx + 6] as i64) & 0xff) << 48;
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            6 => {
                k1 ^= ((data[idx + 5] as i64) & 0xff) << 40;
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            5 => {
                k1 ^= ((data[idx + 4] as i64) & 0xff) << 32;
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            4 => {
                k1 ^= ((data[idx + 3] as i64) & 0xff) << 24;
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            3 => {
                k1 ^= ((data[idx + 2] as i64) & 0xff) << 16;
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            2 => {
                k1 ^= ((data[idx + 1] as i64) & 0xff) << 8;
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            1 => {
                k1 ^= (data[idx] as i64) & 0xff;
                k1 = k1.wrapping_mul(c1);
                k1 = k1.rotate_left(r1);
                k1 = k1.wrapping_mul(c2);
                h1 ^= k1;
            }
            _ => {}
        }

        h1 ^= length as i64;
        h2 ^= length as i64;

        h1 = h1.wrapping_add(h2);
        h2 = h2.wrapping_add(h1);

        h1 = Self::fmix64(h1);
        h2 = Self::fmix64(h2);

        h1 = h1.wrapping_add(h2);
        h2 = h2.wrapping_add(h1);

        [h1, h2]
    }

    /// Returns the 128-bit MurmurHash3 x64 hash of the active slice of `data`.
    pub fn murmurhash3_x64_128_bytes_ref(data: &BytesRef) -> [i64; 2] {
        Self::murmurhash3_x64_128(&data.bytes, data.offset as i32, data.length as i32, 104729)
    }

    fn fmix64(mut hash: i64) -> i64 {
        hash ^= (hash as u64 >> 33) as i64;
        hash = hash.wrapping_mul(0xff51afd7ed558ccdu64 as i64);
        hash ^= (hash as u64 >> 33) as i64;
        hash = hash.wrapping_mul(0xc4ceb9fe1a85ec53u64 as i64);
        hash ^= (hash as u64 >> 33) as i64;
        hash
    }

    /// Generates a non-cryptographic globally unique 16-byte ID.
    ///
    /// Equivalent to `StringHelper.randomId()` (`StringHelper.java:450-479`):
    /// the big-endian bytes of the unsigned 128-bit `nextId`, which is then
    /// incremented modulo 2^128 under the lock. `nextId` is seeded once, as
    /// the static initialiser seeds it (see [`initial_next_id`]).
    pub fn random_id() -> [u8; 16] {
        let mut counter = get_id_counter()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let id = *counter;
        *counter = counter.wrapping_add(1);
        id.to_be_bytes()
    }

    /// Renders an ID as a lowercase hex string, or `(null)` for a `None` input.
    pub fn id_to_string(id: Option<&[u8]>) -> String {
        match id {
            None => "(null)".to_string(),
            Some(id) => {
                let mut s = String::with_capacity(id.len() * 2);
                for b in id {
                    s.push_str(&format!("{:02x}", b));
                }
                if id.len() != ID_LENGTH {
                    s.push_str(" (INVALID FORMAT)");
                }
                s
            }
        }
    }

    /// Converts an [`IntsRef`] to a [`BytesRef`], checking that every value fits
    /// in a byte.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if any int is out of the byte range.
    pub fn ints_ref_to_bytes_ref(ints: &IntsRef) -> Result<BytesRef, LuceneError> {
        let slice = ints.slice();
        let mut bytes = Vec::with_capacity(slice.len());
        for (i, &x) in slice.iter().enumerate() {
            if !(0..=255).contains(&x) {
                return Err(LuceneError::illegal_argument(format!(
                    "int at pos={} with value={} is out-of-bounds for byte",
                    i, x
                )));
            }
            bytes.push(x as u8);
        }
        Ok(BytesRef::new(bytes))
    }
}

/// Length in bytes of an ID generated by [`StringHelper::random_id`].
pub const ID_LENGTH: usize = 16;

/// `StringHelper.nextId`: the unsigned 128-bit value of the next ID.
static ID_COUNTER: OnceLock<Mutex<u128>> = OnceLock::new();

fn get_id_counter() -> &'static Mutex<u128> {
    ID_COUNTER.get_or_init(|| {
        // Java reads the `tests.seed` system property; the crate reads system
        // properties from the environment variable of the same name (see
        // `StringHelper::good_fast_hash_seed`).
        let prop = std::env::var("tests.seed").ok();
        Mutex::new(initial_next_id(prop.as_deref()))
    })
}

/// The static initialiser of `StringHelper.nextId` (`StringHelper.java:370-444`)
/// for the `tests.seed` property `prop`.
///
/// With the property set, "so if there is a test failure that somehow relied
/// on this id, we remain reproducible based on the test seed": both halves of
/// the xorshift128 state are `Long.parseLong` of its last eight characters in
/// radix 16. Without it, the state is the first two big-endian longs of
/// `/dev/urandom`; where that cannot be read, Java falls back to
/// `System.nanoTime()` and to `StringHelper.class.hashCode() << 32` or'ed
/// with the hash code of the concatenated system properties.
///
/// Ten rounds of xorshift128 then scatter the state, and `nextId` is `x0`
/// (high 64 bits) concatenated with `x1` (low 64 bits).
///
/// **Divergence (fallback only).** A Rust process has neither a class
/// identity hash code nor Java's system properties: the fallback `x1` is the
/// process id shifted left by 32, or'ed with the `String.hashCode()` of the
/// concatenated environment variables. It is used only where `/dev/urandom`
/// is unavailable, where Java's value is not reproducible either.
///
/// # Panics
///
/// `java.lang.NumberFormatException` when the last eight characters of
/// `prop` are not a radix-16 long, where Java's class initialisation fails
/// with that exception.
pub fn initial_next_id(prop: Option<&str>) -> u128 {
    // State for xorshift128:
    let (mut x0, mut x1): (i64, i64) = match prop {
        Some(prop) => {
            let units: Vec<u16> = prop.encode_utf16().collect();
            let tail = if units.len() > 8 {
                String::from_utf16_lossy(&units[units.len() - 8..])
            } else {
                prop.to_string()
            };
            let x = java_parse_long_radix16(&tail);
            (x, x)
        }
        None => match read_dev_urandom() {
            Some(pair) => pair,
            None => {
                let x0 = std::time::Instant::now()
                    .duration_since(*NANO_ORIGIN.get_or_init(std::time::Instant::now))
                    .as_nanos() as i64;
                let mut concatenated = String::new();
                for (k, v) in std::env::vars() {
                    concatenated.push_str(&k);
                    concatenated.push_str(&v);
                }
                let x1 = (i64::from(std::process::id() as i32) << 32)
                    | i64::from(java_string_hash_code(&concatenated));
                (x0, x1)
            }
        },
    };
    // Use a few iterations of xorshift128 to scatter the seed in case
    // multiple Lucene instances starting up "near" the same nanoTime, since
    // we use ++ (mod 2^128) for full period cycle:
    for _ in 0..10 {
        let mut s1 = x0;
        let s0 = x1;
        x0 = s0;
        s1 ^= s1 << 23; // a
        x1 = s1 ^ s0 ^ ((s1 as u64 >> 17) as i64) ^ ((s0 as u64 >> 26) as i64); // b, c
    }
    // Concatenate the bits of x0 and x1, as an unsigned 128-bit integer.
    (u128::from(x0 as u64) << 64) | u128::from(x1 as u64)
}

/// The origin of the fallback's `System.nanoTime()`.
static NANO_ORIGIN: OnceLock<std::time::Instant> = OnceLock::new();

/// `new DataInputStream(Files.newInputStream(Paths.get("/dev/urandom")))`
/// read with two `readLong()`s; `None` where Java catches the exception.
fn read_dev_urandom() -> Option<(i64, i64)> {
    use std::io::Read;
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .ok()?;
    let mut hi = [0u8; 8];
    let mut lo = [0u8; 8];
    hi.copy_from_slice(&bytes[..8]);
    lo.copy_from_slice(&bytes[8..]);
    Some((i64::from_be_bytes(hi), i64::from_be_bytes(lo)))
}

/// `Long.parseLong(s, 16)`: an optional `+` or `-` and at least one radix-16
/// digit, the value within the range of a long.
///
/// # Panics
///
/// `java.lang.NumberFormatException` with Java's message
/// (`For input string: "s" under radix 16`) otherwise.
fn java_parse_long_radix16(s: &str) -> i64 {
    let fail = || -> ! {
        std::panic::panic_any(format!(
            "java.lang.NumberFormatException: For input string: \"{s}\" under radix 16"
        ))
    };
    let (negative, digits) = match s.as_bytes().first() {
        Some(b'-') => (true, &s[1..]),
        Some(b'+') => (false, &s[1..]),
        _ => (false, s),
    };
    if digits.is_empty() {
        fail();
    }
    // Accumulate negatively, as `Long.parseLong` does, so that
    // `-8000000000000000` is accepted.
    let mut result: i64 = 0;
    for c in digits.chars() {
        let Some(d) = c.to_digit(16) else { fail() };
        result = match result
            .checked_mul(16)
            .and_then(|r| r.checked_sub(i64::from(d)))
        {
            Some(r) => r,
            None => fail(),
        };
    }
    if negative {
        result
    } else {
        result.checked_neg().unwrap_or_else(|| fail())
    }
}

// ---------------------------------------------------------------------------
// Standalone string serialization helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// String ordering
// ---------------------------------------------------------------------------

/// Compares two strings the way `java.lang.String.compareTo` does.
///
/// Java compares strings by **UTF-16 code unit**, Rust's [`str`] ordering
/// compares them by **UTF-8 byte**. The two agree for every string made of
/// characters below `U+E000`, and disagree above it: a supplementary character
/// (`U+10000` and above) is a surrogate pair in UTF-16, and surrogates occupy
/// `U+D800..=U+DFFF`, which sorts *below* `U+E000..=U+FFFF`; in UTF-8 the same
/// character starts with `0xF0` and sorts *above* it. `"\u{10000}"` therefore
/// sorts before `"\u{FFFF}"` in Java and after it in Rust.
///
/// Lucene orders field names with `String.compareTo` — see
/// `TermsHashPerField.compareTo(TermsHashPerField)` in
/// `lucene/core/src/java/org/apache/lucene/index/TermsHashPerField.java` — and
/// that order is written into the segment, so a port that used Rust's ordering
/// would produce different bytes for such names. Use this comparator wherever
/// an order reaches the index files.
///
/// # Examples
///
/// ```
/// use std::cmp::Ordering;
/// use rucene::util::compare_utf16;
///
/// assert_eq!(compare_utf16("a", "b"), Ordering::Less);
/// // The supplementary character sorts first, exactly as in Java.
/// assert_eq!(compare_utf16("\u{10000}", "\u{FFFF}"), Ordering::Less);
/// // Rust's own ordering disagrees.
/// assert_eq!("\u{10000}".cmp("\u{FFFF}"), Ordering::Greater);
/// ```
pub fn compare_utf16(left: &str, right: &str) -> std::cmp::Ordering {
    // The fast path: while both strings stay inside the Basic Multilingual
    // Plane below `U+E000`, UTF-8 byte order already is UTF-16 order, and
    // `str::cmp` is a `memcmp`. Only strings that actually carry a character at
    // `U+E000` or above pay for the code-unit walk.
    if is_utf16_order_safe(left) && is_utf16_order_safe(right) {
        return left.cmp(right);
    }
    left.encode_utf16().cmp(right.encode_utf16())
}

/// Returns `true` when the UTF-8 order of `text` equals its UTF-16 order
/// against any other such string, i.e. when it holds no character at `U+E000`
/// or above.
///
/// `U+E000` is `0xEE 0x80 0x80` in UTF-8, so every character from `U+E000` up
/// — including every supplementary character, which starts at `0xF0` — has a
/// lead byte of `0xEE` or more.
fn is_utf16_order_safe(text: &str) -> bool {
    !text.as_bytes().iter().any(|byte| *byte >= 0xEE)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_difference_and_sort_key_length() {
        let prior = BytesRef::new(vec![b'a', b'b', b'c']);
        let current = BytesRef::new(vec![b'a', b'b', b'd']);
        assert_eq!(StringHelper::bytes_difference(&prior, &current).unwrap(), 2);
        assert_eq!(StringHelper::sort_key_length(&prior, &current).unwrap(), 3);

        let prior = BytesRef::new(vec![b'a', b'b']);
        let current = BytesRef::new(vec![b'a', b'b', b'c']);
        assert_eq!(StringHelper::bytes_difference(&prior, &current).unwrap(), 2);
        assert_eq!(StringHelper::sort_key_length(&prior, &current).unwrap(), 3);
    }

    /// `Arrays.mismatch` answers `-1` only for equal slices, so only a
    /// duplicate term is refused (`StringHelper.java:51-54`); a prior term that
    /// extends the current one answers the shorter length.
    #[test]
    fn bytes_difference_rejects_only_equal_terms() {
        let prior = BytesRef::new(vec![b'a', b'b']);
        let current = BytesRef::new(vec![b'a', b'b']);
        let error = StringHelper::bytes_difference(&prior, &current).unwrap_err();
        assert!(
            matches!(&error.kind, crate::error::LuceneErrorKind::IllegalArgument { message, .. }
                if message == "terms out of order: priorTerm=[61 62],currentTerm=[61 62]"),
            "{error:?}"
        );
        let prior = BytesRef::new(vec![b'a', b'b', b'c']);
        let current = BytesRef::new(vec![b'a', b'b']);
        assert_eq!(StringHelper::bytes_difference(&prior, &current).unwrap(), 2);
    }

    #[test]
    fn starts_with_and_ends_with() {
        let r = BytesRef::new(vec![b'a', b'b', b'c', b'd']);
        let prefix = BytesRef::new(vec![b'a', b'b']);
        let suffix = BytesRef::new(vec![b'c', b'd']);
        let wrong = BytesRef::new(vec![b'x']);

        assert!(StringHelper::starts_with(&r, &prefix));
        assert!(!StringHelper::starts_with(&r, &wrong));
        assert!(StringHelper::ends_with(&r, &suffix));
        assert!(!StringHelper::ends_with(&r, &wrong));

        assert!(StringHelper::starts_with_bytes(b"abcd", &prefix));
        assert!(StringHelper::starts_with_bytes(b"ab", &prefix));
        assert!(!StringHelper::starts_with_bytes(b"aa", &prefix));
    }

    #[test]
    fn murmurhash3_x86_32_known_values() {
        // Reference vectors verified independently against MurmurHash3 x86 32.
        let data = b"hello";
        assert_eq!(StringHelper::murmurhash3_x86_32(data, 0, 5, 0), 613_153_351);

        let data = b"hello world";
        assert_eq!(
            StringHelper::murmurhash3_x86_32(data, 0, 11, 123),
            679_062_093
        );
    }

    #[test]
    fn murmurhash3_x64_128_matches_bytes_ref() {
        let data = BytesRef::new(b"The quick brown fox jumps over the lazy dog".to_vec());
        let from_ref = StringHelper::murmurhash3_x64_128_bytes_ref(&data);
        let from_slice = StringHelper::murmurhash3_x64_128(
            &data.bytes,
            data.offset as i32,
            data.length as i32,
            104729,
        );
        assert_eq!(from_ref, from_slice);
    }

    #[test]
    fn random_id_is_unique_and_valid_length() {
        let a = StringHelper::random_id();
        let b = StringHelper::random_id();
        assert_eq!(a.len(), ID_LENGTH);
        assert_eq!(b.len(), ID_LENGTH);
        assert_ne!(a, b);
        let s = StringHelper::id_to_string(Some(&a));
        assert!(!s.contains("INVALID FORMAT"));
        assert_eq!(StringHelper::id_to_string(None), "(null)");
    }

    #[test]
    fn ints_ref_to_bytes_ref_round_trip() {
        let ints = IntsRef::new(vec![0, 1, 127, 128, 255]);
        let bytes = StringHelper::ints_ref_to_bytes_ref(&ints).unwrap();
        assert_eq!(bytes.bytes, vec![0u8, 1, 127, 128, 255]);

        let bad = IntsRef::new(vec![256]);
        assert!(StringHelper::ints_ref_to_bytes_ref(&bad).is_err());
    }

    // -- compare_utf16 -------------------------------------------------------

    #[test]
    fn compare_utf16_matches_rust_ordering_below_u_e000() {
        for (left, right) in [
            ("", "a"),
            ("a", "a"),
            ("a", "b"),
            ("abc", "abd"),
            ("abc", "ab"),
            ("\u{7F}", "\u{80}"),
            ("\u{07FF}", "\u{0800}"),
            ("caf\u{e9}", "cafe"),
            ("\u{D7FF}", "\u{E000}"),
        ] {
            assert_eq!(
                compare_utf16(left, right),
                left.cmp(right),
                "{left:?} vs {right:?}"
            );
        }
    }

    #[test]
    fn compare_utf16_orders_supplementary_characters_the_way_java_does() {
        use std::cmp::Ordering;
        // A supplementary character is a surrogate pair in UTF-16, and
        // surrogates sit below `U+E000`; in UTF-8 it starts with `0xF0` and
        // sits above it. Java's `String.compareTo` therefore disagrees with
        // Rust's `str` ordering here.
        assert_eq!(compare_utf16("\u{10000}", "\u{FFFF}"), Ordering::Less);
        assert_eq!("\u{10000}".cmp("\u{FFFF}"), Ordering::Greater);
        assert_eq!(compare_utf16("\u{10FFFF}", "\u{E000}"), Ordering::Less);
        assert_eq!(compare_utf16("\u{FFFF}", "\u{10000}"), Ordering::Greater);
        // Two supplementary characters still compare by code point.
        assert_eq!(compare_utf16("\u{10000}", "\u{10001}"), Ordering::Less);
        // A shared prefix does not disturb the rule.
        assert_eq!(compare_utf16("x\u{10000}", "x\u{FFFF}"), Ordering::Less);
    }

    #[test]
    fn compare_utf16_agrees_with_java_string_compare_to_on_utf16_units() {
        // The reference definition: compare the UTF-16 code units directly.
        let samples = [
            "",
            "a",
            "ab",
            "\u{80}",
            "\u{7FF}",
            "\u{800}",
            "\u{D7FF}",
            "\u{E000}",
            "\u{FFFD}",
            "\u{FFFF}",
            "\u{10000}",
            "\u{1F600}",
            "\u{10FFFF}",
            "a\u{10000}",
            "a\u{FFFF}",
        ];
        for left in samples {
            for right in samples {
                let expected = left.encode_utf16().cmp(right.encode_utf16());
                assert_eq!(
                    compare_utf16(left, right),
                    expected,
                    "{left:?} vs {right:?}"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// StringHelper.GOOD_FAST_HASH_SEED
// ---------------------------------------------------------------------------

static GOOD_FAST_HASH_SEED: OnceLock<i32> = OnceLock::new();

impl StringHelper {
    /// The seed [`BytesRef::hash_code`] and the other `murmurhash3_x86_32`
    /// callers mix in.
    ///
    /// Equivalent to `StringHelper.GOOD_FAST_HASH_SEED`, whose static
    /// initialiser reads the `tests.seed` system property and uses its
    /// `String.hashCode()` when it is set — so that a failure that depended on
    /// hash order stays reproducible — and otherwise takes
    /// `(int) System.currentTimeMillis()`. Rust has no system properties, so
    /// the property is read from the environment variable of the same name,
    /// which is how the rest of this crate reads `tests.seed` (see
    /// `BitUtil::get_native_byte_order`).
    ///
    /// The value is therefore **not** stable across runs unless `tests.seed` is
    /// set, exactly as in Java.
    pub fn good_fast_hash_seed() -> i32 {
        *GOOD_FAST_HASH_SEED.get_or_init(|| match std::env::var("tests.seed") {
            Ok(prop) => java_string_hash_code(&prop),
            Err(_) => SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as i64 as i32)
                .unwrap_or(0),
        })
    }
}

/// `java.lang.String.hashCode()`, which the seeded value above depends on.
fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(i32::from(c));
    }
    h
}

#[cfg(test)]
mod ints_ref_constructor_tests {
    use super::*;

    /// rmp #312: `IntsRef(int[], int, int)` ends with `assert isValid()`
    /// (`IntsRef.java:54-59`), whose `IllegalStateException` names the broken
    /// invariant; the port checks it under `debug_assert!`.
    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "offset+length out of bounds: offset=1,length=2,ints.length=2")]
    fn the_constructor_asserts_is_valid_in_debug_builds() {
        let _ = IntsRef::with_offset_length(vec![1, 2], 1, 2);
    }

    /// A valid reference keeps the given offset and length.
    #[test]
    fn the_constructor_keeps_a_valid_offset_and_length() {
        let r = IntsRef::with_offset_length(vec![1, 2, 3], 1, 2);
        assert_eq!((r.offset, r.length, r.slice()), (1, 2, &[2, 3][..]));
    }
}
