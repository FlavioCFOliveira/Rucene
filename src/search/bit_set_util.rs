//! Bit-set helpers the boolean and conjunction scorers need.
//!
//! # Why this module exists
//!
//! Lucene's scorers call a handful of `org.apache.lucene.util.FixedBitSet`
//! methods — `nextSetBit`, `forEach`, `cardinality(from, to)`,
//! `intoArray`, `and`, `clear(from, to)` and `scanIsEmpty` — and the
//! `Bits.applyMask` default, that this crate's
//! [`FixedBitSet`](crate::util::FixedBitSet) and [`Bits`](crate::util::Bits) do
//! not expose yet. They are transcribed here as free functions so that the
//! scorers in this package read exactly like their Java counterparts.
//!
//! **Divergence from Lucene 10.5.0.** These belong on `FixedBitSet` and `Bits`
//! in `org.apache.lucene.util`; they live here because the port of that package
//! is owned elsewhere. The behaviour is Lucene's, and
//! [`and_in_place`](and_in_place) is the only one whose implementation differs:
//! Java intersects word by word through a package-private mutable view of the
//! backing array, which this crate does not expose, so the intersection clears
//! the bits that do not survive one at a time. The resulting bit set is
//! identical.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::util::{Bits, FixedBitSet};

/// Returns the position of the first set bit at or after `index`, or
/// [`NO_MORE_DOCS`] if there is none.
///
/// Equivalent to `FixedBitSet.nextSetBit(int)`.
pub fn next_set_bit(bit_set: &FixedBitSet, index: usize) -> i32 {
    let words = bit_set.get_bits();
    if index >= bit_set.length() {
        return NO_MORE_DOCS;
    }
    let mut i = index >> 6;
    let word = words[i] >> (index & 63);
    if word != 0 {
        return (index + word.trailing_zeros() as usize) as i32;
    }
    i += 1;
    while i < words.len() {
        if words[i] != 0 {
            return ((i << 6) + words[i].trailing_zeros() as usize) as i32;
        }
        i += 1;
    }
    NO_MORE_DOCS
}

/// Calls `consumer` on `base + i` for every set bit `i` in `[from, to)`.
///
/// Equivalent to `FixedBitSet.forEach(int, int, int, CheckedIntConsumer)`.
///
/// # Errors
///
/// Propagates whatever the consumer fails with, including the
/// [`CollectionTerminated`](CollectionError::CollectionTerminated) signal.
pub fn for_each(
    bit_set: &FixedBitSet,
    from: usize,
    to: usize,
    base: i32,
    consumer: &mut dyn FnMut(i32) -> std::result::Result<(), CollectionError>,
) -> CollectionResult<()> {
    let to = to.min(bit_set.length());
    let mut doc = next_set_bit(bit_set, from);
    while doc != NO_MORE_DOCS && (doc as usize) < to {
        consumer(base + doc)?;
        let next = doc as usize + 1;
        if next >= bit_set.length() {
            break;
        }
        doc = next_set_bit(bit_set, next);
    }
    Ok(())
}

/// Returns the number of set bits in `[from, to)`.
///
/// Equivalent to `FixedBitSet.cardinality(int, int)`.
pub fn cardinality_range(bit_set: &FixedBitSet, from: usize, to: usize) -> i32 {
    let to = to.min(bit_set.length());
    if from >= to {
        return 0;
    }
    let words = bit_set.get_bits();
    let first_word = from >> 6;
    let last_word = (to - 1) >> 6;
    if first_word == last_word {
        let mask = mask_between(from & 63, ((to - 1) & 63) + 1);
        return (words[first_word] & mask).count_ones() as i32;
    }
    let mut count = (words[first_word] & mask_from(from & 63)).count_ones();
    for word in &words[first_word + 1..last_word] {
        count += word.count_ones();
    }
    count += (words[last_word] & mask_to(((to - 1) & 63) + 1)).count_ones();
    count as i32
}

/// Copies up to `array.len()` set bits of `[from, to)` into `array`, offset by
/// `base`, and returns how many were copied.
///
/// Equivalent to `FixedBitSet.intoArray(int, int, int, int[])`.
pub fn into_array(
    bit_set: &FixedBitSet,
    from: usize,
    to: usize,
    base: i32,
    array: &mut [i32],
) -> usize {
    let to = to.min(bit_set.length());
    let mut count = 0;
    if from >= to || array.is_empty() {
        return 0;
    }
    let mut doc = next_set_bit(bit_set, from);
    while doc != NO_MORE_DOCS && (doc as usize) < to && count < array.len() {
        array[count] = base + doc;
        count += 1;
        let next = doc as usize + 1;
        if next >= bit_set.length() {
            break;
        }
        doc = next_set_bit(bit_set, next);
    }
    count
}

/// Intersects `bit_set` with `other` in place.
///
/// Equivalent to `FixedBitSet.and(FixedBitSet)`; see the module documentation
/// for why the loop clears bit by bit.
pub fn and_in_place(bit_set: &mut FixedBitSet, other: &FixedBitSet) {
    let length = bit_set.length();
    let mut doc = next_set_bit(bit_set, 0);
    while doc != NO_MORE_DOCS {
        let index = doc as usize;
        if index >= other.length() || !other.get(index) {
            bit_set.clear(index);
        }
        if index + 1 >= length {
            break;
        }
        doc = next_set_bit(bit_set, index + 1);
    }
}

/// Clears the bits in `[from, to)`.
///
/// Equivalent to `FixedBitSet.clear(int, int)`.
pub fn clear_range(bit_set: &mut FixedBitSet, from: usize, to: usize) {
    let to = to.min(bit_set.length());
    let mut index = from;
    while index < to {
        let doc = next_set_bit(bit_set, index);
        if doc == NO_MORE_DOCS || (doc as usize) >= to {
            break;
        }
        bit_set.clear(doc as usize);
        index = doc as usize + 1;
    }
}

/// Returns whether no bit is set.
///
/// Equivalent to `FixedBitSet.scanIsEmpty()`.
pub fn scan_is_empty(bit_set: &FixedBitSet) -> bool {
    bit_set.get_bits().iter().all(|word| *word == 0)
}

/// Clears the bits of `bit_set` whose document, once re-based by `offset`, is
/// not accepted.
///
/// Equivalent to the `Bits.applyMask(FixedBitSet, int)` default implementation.
pub fn apply_mask(accept_docs: &dyn Bits, bit_set: &mut FixedBitSet, offset: i32) {
    let length = bit_set.length();
    let mut doc = next_set_bit(bit_set, 0);
    while doc != NO_MORE_DOCS {
        let index = doc as usize;
        if !accept_docs.get((offset + doc) as usize) {
            bit_set.clear(index);
        }
        if index + 1 >= length {
            break;
        }
        doc = next_set_bit(bit_set, index + 1);
    }
}

/// Copies the set bits of `[from, to)` into a vector, offset by `base`.
///
/// The allocation-free `for_each` is preferred; this exists for the callers
/// that need to walk the matches while mutating the bit set.
///
/// # Errors
///
/// Never fails; the signature mirrors [`for_each`] so that call sites read the
/// same.
pub fn collect_set_bits(bit_set: &FixedBitSet, from: usize, to: usize, base: i32) -> Result<Vec<i32>> {
    let mut docs = Vec::new();
    let to = to.min(bit_set.length());
    let mut doc = next_set_bit(bit_set, from);
    while doc != NO_MORE_DOCS && (doc as usize) < to {
        docs.push(base + doc);
        let next = doc as usize + 1;
        if next >= bit_set.length() {
            break;
        }
        doc = next_set_bit(bit_set, next);
    }
    Ok(docs)
}

/// A mask with bits `[from, 64)` set.
fn mask_from(from: usize) -> u64 {
    u64::MAX << from
}

/// A mask with bits `[0, to)` set.
fn mask_to(to: usize) -> u64 {
    if to >= 64 {
        u64::MAX
    } else {
        (1u64 << to) - 1
    }
}

/// A mask with bits `[from, to)` set.
fn mask_between(from: usize, to: usize) -> u64 {
    mask_from(from) & mask_to(to)
}
