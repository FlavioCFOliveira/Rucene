//! String and byte-reference helpers ported from `org.apache.lucene.util.StringHelper`.
//!
//! This module also provides standalone [`write_string`] and [`read_string`] helpers
//! that produce/consume the same byte serialization used by Lucene's
//! `DataOutput.writeString` / `DataInput.readString`: a VInt length followed by
//! UTF-8 bytes. These functions are kept inside `util` so they do not introduce a
//! dependency from `util` on the `store` module.

#![deny(unsafe_code)]

use std::{
    io,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use crate::{error::LuceneError, util::BitUtil};

use super::BytesRef;

// ---------------------------------------------------------------------------
// IntsRef
// ---------------------------------------------------------------------------

/// A reference to a slice of `i32` values, equivalent to Lucene's `IntsRef`.
#[derive(Clone, Default, Debug)]
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

    /// Returns the active slice.
    pub fn slice(&self) -> &[i32] {
        &self.ints[self.offset..self.offset + self.length]
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
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `current_term` is not strictly
    /// after `prior_term` (i.e., they are equal or out of order).
    pub fn bytes_difference(
        prior_term: &BytesRef,
        current_term: &BytesRef,
    ) -> Result<i32, LuceneError> {
        let a = prior_term.slice();
        let b = current_term.slice();
        let min_len = a.len().min(b.len());
        let mut mismatch = min_len;
        for i in 0..min_len {
            if a[i] != b[i] {
                mismatch = i;
                break;
            }
        }
        if mismatch == min_len && a.len() > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "terms out of order: priorTerm={:?}, currentTerm={:?}",
                prior_term, current_term
            )));
        }
        mismatch
            .try_into()
            .map_err(|_| LuceneError::IllegalState("mismatch index overflow".to_string()))
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
    /// The implementation is functionally equivalent to Lucene's `randomId()`:
    /// a 128-bit counter seeded from the system clock, incremented under a
    /// mutex to guarantee uniqueness within a process.
    pub fn random_id() -> [u8; 16] {
        let mut counter = get_id_counter().lock().expect("ID counter mutex poisoned");
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
    /// Returns `LuceneError::IllegalArgument` if any int is out of the byte range.
    pub fn ints_ref_to_bytes_ref(ints: &IntsRef) -> Result<BytesRef, LuceneError> {
        let slice = ints.slice();
        let mut bytes = Vec::with_capacity(slice.len());
        for (i, &x) in slice.iter().enumerate() {
            if !(0..=255).contains(&x) {
                return Err(LuceneError::IllegalArgument(format!(
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

static ID_COUNTER: OnceLock<Mutex<u128>> = OnceLock::new();

fn get_id_counter() -> &'static Mutex<u128> {
    ID_COUNTER.get_or_init(|| {
        let seed = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Mutex::new(seed)
    })
}

// Initialize the static reference eagerly so `random_id()` can use it directly.

// ---------------------------------------------------------------------------
// Standalone string serialization helpers
// ---------------------------------------------------------------------------

/// Encodes a string as a VInt length followed by UTF-8 bytes.
///
/// This matches the byte output of `DataOutput.writeString` in Lucene.
pub fn write_string(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len() + 5);
    write_v_int(bytes.len() as i32, &mut out);
    out.extend_from_slice(bytes);
    out
}

/// Decodes a string previously encoded with [`write_string`].
///
/// # Errors
///
/// Returns `LuceneError::Io` if the VInt is truncated and
/// `LuceneError::IllegalArgument` if the UTF-8 bytes are invalid.
pub fn read_string(bytes: &[u8], pos: &mut usize) -> Result<String, LuceneError> {
    let length = read_v_int(bytes, pos)? as usize;
    if *pos + length > bytes.len() {
        return Err(LuceneError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "truncated string bytes",
        )));
    }
    let s = String::from_utf8(bytes[*pos..*pos + length].to_vec())
        .map_err(|e| LuceneError::IllegalArgument(format!("invalid UTF-8 reading string: {e}")))?;
    *pos += length;
    Ok(s)
}

fn read_byte(src: &[u8], pos: &mut usize) -> Result<u8, LuceneError> {
    if *pos >= src.len() {
        return Err(LuceneError::Io(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "unexpected EOF reading byte",
        )));
    }
    let b = src[*pos];
    *pos += 1;
    Ok(b)
}

fn read_v_int(src: &[u8], pos: &mut usize) -> Result<i32, LuceneError> {
    let mut b = read_byte(src, pos)? as i32;
    let mut i = b & 0x7F;
    let mut shift = 7;
    while (b & 0x80) != 0 {
        b = read_byte(src, pos)? as i32;
        i |= (b & 0x7F) << shift;
        shift += 7;
    }
    Ok(i)
}

fn write_v_int(mut value: i32, dst: &mut Vec<u8>) {
    while (value & !0x7F) != 0 {
        dst.push((0x80 | (value & 0x7F)) as u8);
        value = ((value as u32) >> 7) as i32;
    }
    dst.push(value as u8);
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

    #[test]
    fn bytes_difference_rejects_out_of_order() {
        let prior = BytesRef::new(vec![b'a', b'b', b'c']);
        let current = BytesRef::new(vec![b'a', b'b']);
        assert!(StringHelper::bytes_difference(&prior, &current).is_err());
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

    #[test]
    fn write_and_read_string_matches_java_format() {
        let cases = [
            ("", vec![0x00]),
            ("hello", vec![0x05, b'h', b'e', b'l', b'l', b'o']),
            ("héllo", vec![0x06, b'h', 0xc3, 0xa9, b'l', b'l', b'o']),
            (
                "日本語",
                vec![0x09, 0xe6, 0x97, 0xa5, 0xe6, 0x9c, 0xac, 0xe8, 0xaa, 0x9e],
            ),
        ];
        for (s, expected) in cases {
            let encoded = write_string(s);
            assert_eq!(encoded, expected, "Java byte mismatch for {:?}", s);
            let mut pos = 0;
            let decoded = read_string(&encoded, &mut pos).unwrap();
            assert_eq!(decoded, s);
            assert_eq!(pos, encoded.len());
        }
    }

    #[test]
    fn read_string_detects_invalid_utf8() {
        let encoded = vec![0x02, 0xc3, 0x28]; // invalid UTF-8
        let mut pos = 0;
        assert!(read_string(&encoded, &mut pos).is_err());
    }
}
