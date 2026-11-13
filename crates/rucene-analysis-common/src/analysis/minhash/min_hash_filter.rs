/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Generates min hash tokens from an incoming stream of tokens, ported from
//! `org.apache.lucene.analysis.minhash.MinHashFilter`.

use std::cmp::Ordering;
use std::collections::BTreeSet;
use std::sync::LazyLock;

use rucene::analysis::{
    CharTermAttribute, OffsetAttribute, PositionIncrementAttribute, PositionLengthAttribute,
    TokenFilter, TokenFilterLogic, TokenStream, TypeAttribute,
};
use rucene::error::{LuceneError, Result};
use rucene::util::attribute::{AttributeSource, CapturedState};

use crate::jdk::character::{chars_to_utf16, utf16_to_term_chars};

/// Equivalent to the private `MinHashFilter.HASH_CACHE_SIZE`.
const HASH_CACHE_SIZE: i32 = 512;

/// Equivalent to `MinHashFilter.DEFAULT_HASH_COUNT`.
pub const DEFAULT_HASH_COUNT: i32 = 1;

/// Equivalent to `MinHashFilter.DEFAULT_HASH_SET_SIZE`.
pub const DEFAULT_HASH_SET_SIZE: i32 = 1;

/// Equivalent to `MinHashFilter.DEFAULT_BUCKET_COUNT`.
pub const DEFAULT_BUCKET_COUNT: i32 = 512;

/// Equivalent to the package-private `MinHashFilter.MIN_HASH_TYPE`.
pub(crate) const MIN_HASH_TYPE: &str = "MIN_HASH";

/// Equivalent to the private static `cachedIntHashes`, filled by the
/// class's static initializer.
static CACHED_INT_HASHES: LazyLock<Vec<LongPair>> = LazyLock::new(|| {
    (0..HASH_CACHE_SIZE)
        .map(|i| murmurhash3_x64_128(&get_bytes(i), 0, 4, 0))
        .collect()
});

/// Equivalent to the package-private static `getBytes(int)`: `i` in
/// big-endian order (`BitUtil.VH_BE_INT`).
pub(crate) fn get_bytes(i: i32) -> [u8; 4] {
    i.to_be_bytes()
}

/// The body of [`MinHashFilter`]: what Lucene's `MinHashFilter` adds to
/// `TokenFilter`.
///
/// **Divergence from Lucene 10.5.0.** A min hash term is a sequence of
/// arbitrary 16-bit values, which Java appends to the term's `char[]` as
/// they are; `rucene`'s term buffer holds `char`s, so the units are decoded
/// into it, a value that is an unpaired surrogate becoming U+FFFD (see
/// `jdk::character::utf16_to_term_chars`).
#[derive(Debug)]
pub struct MinHashFilterLogic {
    min_hash_sets: Vec<Vec<FixedSizeTreeSet<LongPair>>>,
    hash_set_size: i32,
    bucket_count: i32,
    hash_count: i32,
    requires_initialisation: bool,
    end_state: Option<CapturedState>,
    hash_position: i32,
    bucket_position: i32,
    bucket_size: i64,
    with_rotation: bool,
    end_offset: i32,
    exhausted: bool,
}

impl MinHashFilterLogic {
    /// Equivalent to the private `doRest()`.
    fn do_rest(&mut self) {
        for i in 0..self.hash_count as usize {
            for j in 0..self.bucket_count as usize {
                self.min_hash_sets[i][j].clear();
            }
        }
        self.end_state = None;
        self.hash_position = -1;
        self.bucket_position = -1;
        self.requires_initialisation = true;
        self.exhausted = false;
    }
}

/// Equivalent to the private static `getIntHash(int)`.
fn get_int_hash(i: i32) -> LongPair {
    if i < HASH_CACHE_SIZE {
        CACHED_INT_HASHES[i as usize]
    } else {
        murmurhash3_x64_128(&get_bytes(i), 0, 4, 0)
    }
}

impl TokenFilterLogic for MinHashFilterLogic {
    /// Equivalent to the final `MinHashFilter.incrementToken()`.
    fn increment_token(
        &mut self,
        source: &mut AttributeSource,
        input: &mut dyn TokenStream,
    ) -> Result<bool> {
        // Pull the underlying stream of tokens
        // Hash each token found
        // Generate the required number of variants of this hash
        // Keep the minimum hash value found so far of each variant

        let mut position_increment = 0;
        if self.requires_initialisation {
            self.requires_initialisation = false;
            let mut found = false;
            // First time through so we pull and hash everything
            while input.increment_token()? {
                found = true;
                let current = {
                    let term = source
                        .get_attribute_interface::<dyn CharTermAttribute>()
                        .expect("INVARIANT: the constructor added the CharTermAttribute");
                    chars_to_utf16(&term.buffer()[..term.length()])
                };
                for i in 0..self.hash_count {
                    // `current.getBytes(StandardCharsets.UTF_16LE)`
                    let bytes: Vec<u8> = current.iter().flat_map(|u| u.to_le_bytes()).collect();
                    let hash = murmurhash3_x64_128(&bytes, 0, bytes.len() as i32, 0);
                    let rehashed = combine_ordered(&[hash, get_int_hash(i)]);
                    let bucket = (((rehashed.val2 as u64) >> 32) as i64 / self.bucket_size) as i32;
                    self.min_hash_sets[i as usize][bucket as usize].add(rehashed);
                }
                self.end_offset = source
                    .get_attribute_interface::<dyn OffsetAttribute>()
                    .expect("INVARIANT: the constructor added the OffsetAttribute")
                    .end_offset();
            }
            self.exhausted = true;
            input.end()?;
            // We need the end state so an underlying shingle filter can have its state restored
            // correctly.
            self.end_state = source.capture_state();
            if !found {
                return Ok(false);
            }

            position_increment = 1;
            // fix up any wrap around bucket values. ...
            if self.with_rotation && (self.hash_set_size == 1) {
                for hash_loop in 0..self.hash_count as usize {
                    for bucket_loop in 0..self.bucket_count {
                        if self.min_hash_sets[hash_loop][bucket_loop as usize].size() == 0 {
                            for bucket_offset in 1..self.bucket_count {
                                let other =
                                    ((bucket_loop + bucket_offset) % self.bucket_count) as usize;
                                if self.min_hash_sets[hash_loop][other].size() > 0 {
                                    let replacement_hash = self.min_hash_sets[hash_loop][other]
                                        .first()
                                        .expect("INVARIANT: the set is not empty");
                                    self.min_hash_sets[hash_loop][bucket_loop as usize]
                                        .add(replacement_hash);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }

        source.clear_attributes();

        while self.hash_position < self.hash_count {
            if self.hash_position == -1 {
                self.hash_position += 1;
            } else {
                while self.bucket_position < self.bucket_count {
                    if self.bucket_position == -1 {
                        self.bucket_position += 1;
                    } else {
                        let hash = self.min_hash_sets[self.hash_position as usize]
                            [self.bucket_position as usize]
                            .poll_first();
                        if let Some(hash) = hash {
                            let mut units: Vec<u16> = Vec::with_capacity(8);
                            if self.hash_count > 1 {
                                units.push(int0(self.hash_position));
                                units.push(int1(self.hash_position));
                            }
                            let high = hash.val2;
                            units.push(long0(high));
                            units.push(long1(high));
                            units.push(long2(high));
                            units.push(long3(high));
                            let low = hash.val1;
                            units.push(long0(low));
                            units.push(long1(low));
                            if self.hash_count == 1 {
                                units.push(long2(low));
                                units.push(long3(low));
                            }
                            let chars = utf16_to_term_chars(&units);
                            source
                                .get_attribute_interface_mut::<dyn CharTermAttribute>()
                                .expect("INVARIANT: the constructor added the CharTermAttribute")
                                .copy_buffer(&chars, 0, chars.len());
                            source
                                .get_attribute_interface_mut::<dyn PositionIncrementAttribute>()
                                .expect(
                                    "INVARIANT: the constructor added the PositionIncrementAttribute",
                                )
                                .set_position_increment(position_increment);
                            source
                                .get_attribute_interface_mut::<dyn OffsetAttribute>()
                                .expect("INVARIANT: the constructor added the OffsetAttribute")
                                .set_offset(0, self.end_offset);
                            source
                                .get_attribute_interface_mut::<dyn TypeAttribute>()
                                .expect("INVARIANT: the constructor added the TypeAttribute")
                                .set_type_static(MIN_HASH_TYPE);
                            source
                                .get_attribute_interface_mut::<dyn PositionLengthAttribute>()
                                .expect(
                                    "INVARIANT: the constructor added the PositionLengthAttribute",
                                )
                                .set_position_length(1);
                            return Ok(true);
                        } else {
                            self.bucket_position += 1;
                        }
                    }
                }
                self.bucket_position = -1;
                self.hash_position += 1;
            }
        }
        Ok(false)
    }

    /// Equivalent to `MinHashFilter.end()`.
    fn end(&mut self, source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        if !self.exhausted {
            input.end()?;
        }

        if let Some(end_state) = &self.end_state {
            source.restore_state(end_state)?;
        }
        Ok(())
    }

    /// Equivalent to `MinHashFilter.reset()`.
    fn reset(&mut self, _source: &mut AttributeSource, input: &mut dyn TokenStream) -> Result<()> {
        input.reset()?;
        self.do_rest();
        Ok(())
    }
}

/// Generate min hash tokens from an incoming stream of tokens. The incoming
/// tokens would typically be 5 word shingles.
///
/// The number of hashes used and the number of minimum values for each hash
/// can be set. You could have 1 hash and keep the 100 lowest values or 100
/// hashes and keep the lowest one for each. Hashes can also be bucketed in
/// ranges over the 128-bit hash space,
///
/// A 128-bit hash is used internally. 5 word shingles from 10e5 words
/// generate 10e25 combinations So a 64 bit hash would have collisions
/// (1.8e19)
///
/// When using different hashes 32 bits are used for the hash position
/// leaving scope for 8e28 unique hashes. A single hash will use all 128
/// bits.
///
/// Equivalent to `org.apache.lucene.analysis.minhash.MinHashFilter`.
pub type MinHashFilter = TokenFilter<MinHashFilterLogic>;

/// create a MinHash filter
///
/// * `input` the token stream
/// * `hash_count` the no. of hashes
/// * `bucket_count` the no. of buckets for hashing
/// * `hash_set_size` the no. of min hashes to keep
/// * `with_rotation` whether rotate or not hashes while incrementing tokens
///
/// Equivalent to `MinHashFilter(TokenStream, int, int, int, boolean)`, which
/// adds the `CharTermAttribute`, `OffsetAttribute`, `TypeAttribute`,
/// `PositionIncrementAttribute` and `PositionLengthAttribute`.
///
/// # Errors
///
/// What the stream's attribute factory raises for an interface it cannot
/// implement, as Java's `addAttribute` throws, and
/// `IllegalArgumentException` "hashCount must be greater than zero",
/// "bucketCount must be greater than zero" or "hashSetSize must be greater
/// than zero".
pub fn new_min_hash_filter(
    mut input: Box<dyn TokenStream>,
    hash_count: i32,
    bucket_count: i32,
    hash_set_size: i32,
    with_rotation: bool,
) -> Result<MinHashFilter> {
    // Java's field initializers add the attributes before the constructor
    // body checks its arguments; the filter's source shares the input's
    // attributes, so they are added to the input's source here, before the
    // filter that needs the checked counts is built.
    let source = input.attribute_source_mut();
    source.add_attribute_interface::<dyn CharTermAttribute>()?;
    source.add_attribute_interface::<dyn OffsetAttribute>()?;
    source.add_attribute_interface::<dyn TypeAttribute>()?;
    source.add_attribute_interface::<dyn PositionIncrementAttribute>()?;
    source.add_attribute_interface::<dyn PositionLengthAttribute>()?;
    if hash_count <= 0 {
        return Err(LuceneError::illegal_argument(
            "hashCount must be greater than zero",
        ));
    }
    if bucket_count <= 0 {
        return Err(LuceneError::illegal_argument(
            "bucketCount must be greater than zero",
        ));
    }
    if hash_set_size <= 0 {
        return Err(LuceneError::illegal_argument(
            "hashSetSize must be greater than zero",
        ));
    }
    let mut bucket_size = (1i64 << 32) / i64::from(bucket_count);
    if (1i64 << 32) % i64::from(bucket_count) != 0 {
        bucket_size += 1;
    }
    let mut min_hash_sets = Vec::with_capacity(hash_count as usize);
    for _ in 0..hash_count {
        let mut buckets = Vec::with_capacity(bucket_count as usize);
        for _ in 0..bucket_count {
            buckets.push(FixedSizeTreeSet::with_capacity(hash_set_size));
        }
        min_hash_sets.push(buckets);
    }
    let mut logic = MinHashFilterLogic {
        min_hash_sets,
        hash_set_size,
        bucket_count,
        hash_count,
        requires_initialisation: true,
        end_state: None,
        hash_position: -1,
        bucket_position: -1,
        bucket_size,
        with_rotation,
        end_offset: 0,
        exhausted: false,
    };
    logic.do_rest();
    Ok(TokenFilter::new(input, logic))
}

/// `(char) (x >> 48)`.
fn long0(x: i64) -> u16 {
    (x >> 48) as u16
}

/// `(char) (x >> 32)`.
fn long1(x: i64) -> u16 {
    (x >> 32) as u16
}

/// `(char) (x >> 16)`.
fn long2(x: i64) -> u16 {
    (x >> 16) as u16
}

/// `(char) x`.
fn long3(x: i64) -> u16 {
    x as u16
}

/// `(char) (x >> 16)`.
fn int0(x: i32) -> u16 {
    (x >> 16) as u16
}

/// `(char) x`.
fn int1(x: i32) -> u16 {
    x as u16
}

/// Equivalent to the package-private static `isLessThanUnsigned(long,
/// long)`.
pub(crate) fn is_less_than_unsigned(n1: i64, n2: i64) -> bool {
    (n1 < n2) ^ ((n1 < 0) != (n2 < 0))
}

/// A sorted set holding at most `capacity` elements, the smallest added.
///
/// Equivalent to `org.apache.lucene.analysis.minhash.MinHashFilter$FixedSizeTreeSet`,
/// a `TreeSet`: the set is a [`BTreeSet`], ordered as the elements'
/// `compareTo` orders them.
#[derive(Debug, Clone)]
pub(crate) struct FixedSizeTreeSet<E: Ord> {
    set: BTreeSet<E>,
    capacity: i32,
}

impl<E: Ord + Clone> FixedSizeTreeSet<E> {
    /// Equivalent to `FixedSizeTreeSet()`, `this(20)`.
    #[allow(dead_code)]
    pub(crate) fn new() -> Self {
        Self::with_capacity(20)
    }

    /// Equivalent to `FixedSizeTreeSet(int)`.
    pub(crate) fn with_capacity(capacity: i32) -> Self {
        Self {
            set: BTreeSet::new(),
            capacity,
        }
    }

    /// Equivalent to `FixedSizeTreeSet.add(E)`: past the capacity, an
    /// element is added only if it is below the greatest, which it evicts.
    pub(crate) fn add(&mut self, to_add: E) -> bool {
        if self.capacity as usize <= self.set.len() {
            let last_elm = self.set.last().expect("INVARIANT: the set is full");
            if to_add.cmp(last_elm) != Ordering::Less {
                return false;
            } else {
                self.set.pop_last();
            }
        }
        self.set.insert(to_add)
    }

    /// `TreeSet.size()`.
    pub(crate) fn size(&self) -> usize {
        self.set.len()
    }

    /// `TreeSet.first()`, `None` where Java throws `NoSuchElementException`.
    pub(crate) fn first(&self) -> Option<E> {
        self.set.first().cloned()
    }

    /// `TreeSet.pollFirst()`.
    pub(crate) fn poll_first(&mut self) -> Option<E> {
        self.set.pop_first()
    }

    /// `TreeSet.clear()`.
    pub(crate) fn clear(&mut self) {
        self.set.clear();
    }
}

/// Equivalent to the private static `combineOrdered(LongPair...)`.
fn combine_ordered(hash_codes: &[LongPair]) -> LongPair {
    let mut result = LongPair::default();
    for hash_code in hash_codes {
        result.val1 = result.val1.wrapping_mul(37).wrapping_add(hash_code.val1);
        result.val2 = result.val2.wrapping_mul(37).wrapping_add(hash_code.val2);
    }
    result
}

/// 128 bits of state
///
/// Equivalent to `org.apache.lucene.analysis.minhash.MinHashFilter$LongPair`:
/// its `compareTo` is [`Ord`] (unsigned, `val2` first), its `equals`
/// [`PartialEq`], and its `hashCode` [`hash_code`](Self::hash_code).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct LongPair {
    pub(crate) val1: i64,
    pub(crate) val2: i64,
}

impl LongPair {
    /// Equivalent to `LongPair.hashCode()`.
    #[allow(dead_code)]
    pub(crate) fn hash_code(&self) -> i32 {
        let mut result = (self.val1 ^ ((self.val1 as u64) >> 32) as i64) as i32;
        result = result
            .wrapping_mul(31)
            .wrapping_add((self.val2 ^ ((self.val2 as u64) >> 32) as i64) as i32);
        result
    }
}

impl Ord for LongPair {
    /// Equivalent to `LongPair.compareTo(LongPair)`.
    fn cmp(&self, other: &Self) -> Ordering {
        if is_less_than_unsigned(self.val2, other.val2) {
            Ordering::Less
        } else if self.val2 == other.val2 {
            if is_less_than_unsigned(self.val1, other.val1) {
                Ordering::Less
            } else if self.val1 == other.val1 {
                Ordering::Equal
            } else {
                Ordering::Greater
            }
        } else {
            Ordering::Greater
        }
    }
}

impl PartialOrd for LongPair {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Gets a long from a byte buffer in little endian byte order.
///
/// Equivalent to the private static `getLongLittleEndian(byte[], int)`.
fn get_long_little_endian(buf: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(
        buf[offset..offset + 8]
            .try_into()
            .expect("INVARIANT: eight bytes"),
    )
}

/// Returns the MurmurHash3_x64_128 hash, placing the result in "out".
///
/// Equivalent to the package-private static `murmurhash3_x64_128(byte[],
/// int, int, int, LongPair)`, returning `out`.
pub(crate) fn murmurhash3_x64_128(key: &[u8], offset: i32, len: i32, seed: i32) -> LongPair {
    // The original algorithm does have a 32 bit unsigned seed.
    // We have to mask to match the behavior of the unsigned types and prevent sign extension.
    let mut h1: i64 = i64::from(seed) & 0x0000_0000_FFFF_FFFF;
    let mut h2: i64 = i64::from(seed) & 0x0000_0000_FFFF_FFFF;

    const C1: i64 = 0x87c3_7b91_1142_53d5_u64 as i64;
    const C2: i64 = 0x4cf5_ad43_2745_937f_u64 as i64;

    let rounded_end = offset + (len & 0xFFFF_FFF0_u32 as i32); // round down to 16 byte block
    let mut i = offset;
    while i < rounded_end {
        let mut k1 = get_long_little_endian(key, i as usize);
        let mut k2 = get_long_little_endian(key, i as usize + 8);
        k1 = k1.wrapping_mul(C1);
        k1 = k1.rotate_left(31);
        k1 = k1.wrapping_mul(C2);
        h1 ^= k1;
        h1 = h1.rotate_left(27);
        h1 = h1.wrapping_add(h2);
        h1 = h1.wrapping_mul(5).wrapping_add(0x52dc_e729);
        k2 = k2.wrapping_mul(C2);
        k2 = k2.rotate_left(33);
        k2 = k2.wrapping_mul(C1);
        h2 ^= k2;
        h2 = h2.rotate_left(31);
        h2 = h2.wrapping_add(h1);
        h2 = h2.wrapping_mul(5).wrapping_add(0x3849_5ab5);
        i += 16;
    }

    let mut k1: i64 = 0;
    let mut k2: i64 = 0;

    let tail = |index: i32| -> i64 { i64::from(key[(rounded_end + index) as usize]) & 0xff };
    // The huge switch is designed to use fall through into cases: the
    // cases a `len & 15` reaches run in order.
    let rem = len & 15;
    if rem >= 15 {
        k2 = tail(14) << 48;
    }
    if rem >= 14 {
        k2 |= tail(13) << 40;
    }
    if rem >= 13 {
        k2 |= tail(12) << 32;
    }
    if rem >= 12 {
        k2 |= tail(11) << 24;
    }
    if rem >= 11 {
        k2 |= tail(10) << 16;
    }
    if rem >= 10 {
        k2 |= tail(9) << 8;
    }
    if rem >= 9 {
        k2 |= tail(8);
        k2 = k2.wrapping_mul(C2);
        k2 = k2.rotate_left(33);
        k2 = k2.wrapping_mul(C1);
        h2 ^= k2;
    }
    if rem >= 8 {
        // `((long) key[roundedEnd + 7]) << 56`: the byte sign-extends, and
        // the shift keeps its low eight bits.
        k1 = i64::from(key[(rounded_end + 7) as usize] as i8) << 56;
    }
    if rem >= 7 {
        k1 |= tail(6) << 48;
    }
    if rem >= 6 {
        k1 |= tail(5) << 40;
    }
    if rem >= 5 {
        k1 |= tail(4) << 32;
    }
    if rem >= 4 {
        k1 |= tail(3) << 24;
    }
    if rem >= 3 {
        k1 |= tail(2) << 16;
    }
    if rem >= 2 {
        k1 |= tail(1) << 8;
    }
    if rem >= 1 {
        k1 |= tail(0);
        k1 = k1.wrapping_mul(C1);
        k1 = k1.rotate_left(31);
        k1 = k1.wrapping_mul(C2);
        h1 ^= k1;
    }

    // ----------
    // finalization

    h1 ^= i64::from(len);
    h2 ^= i64::from(len);

    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);

    h1 = fmix64(h1);
    h2 = fmix64(h2);

    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);

    LongPair { val1: h1, val2: h2 }
}

/// Equivalent to the private static `fmix64(long)`.
fn fmix64(mut k: i64) -> i64 {
    k ^= ((k as u64) >> 33) as i64;
    k = k.wrapping_mul(0xff51_afd7_ed55_8ccd_u64 as i64);
    k ^= ((k as u64) >> 33) as i64;
    k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53_u64 as i64);
    k ^= ((k as u64) >> 33) as i64;
    k
}
