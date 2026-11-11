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

//! Port of `org.apache.lucene.codecs.bloom.FuzzySet`.

use std::fmt;
use std::sync::atomic::Ordering;

use rucene::error::{LuceneError, Result};
use rucene::search::NO_MORE_DOCS;
use rucene::store::{DataInput, DataOutput};
use rucene::util::bit_set::BitSet;
use rucene::util::string_helper::StringHelper;
use rucene::util::{Accountable, BytesRef, FixedBitSet, RamUsageEstimator};

/// Result from [`FuzzySet::contains`]: can never return definitively YES
/// (always MAYBE), but can sometimes definitely return NO.
///
/// Port of `FuzzySet.ContainsResult`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)]
pub enum ContainsResult {
    /// The value may be in the set.
    MAYBE,
    /// The value is definitely not in the set.
    NO,
}

/// The sizes of BitSet used are all numbers that, when expressed in binary
/// form, are all ones. This is to enable fast downsizing from one bitset to
/// another by simply ANDing each set index in one bitset with the size of
/// the target bitset - this provides a fast modulo of the number. Values
/// previously accumulated in a large bitset and then mapped to a smaller set
/// can be looked up using a single AND operation of the query term's hash
/// rather than needing to perform a 2-step translation of the query term
/// that mirrors the stored content's reprojections.
///
/// Port of the static `usableBitSetSizes`: `(1 << (i + 6)) - 1` for `i` in
/// `0..26`.
const USABLE_BIT_SET_SIZES: [i32; 26] = {
    let mut sizes = [0i32; 26];
    let mut i = 0;
    while i < sizes.len() {
        // `(1 << 31) - 1` wraps to `Integer.MAX_VALUE` in Java.
        sizes[i] = (1i32 << (i + 6)).wrapping_sub(1);
        i += 1;
    }
    sizes
};

/// Java's `Math.round(double)`: the closest `long`, ties rounding up
/// (`floor(a + 0.5)` computed exactly), `NaN` to 0, saturating.
fn java_math_round(a: f64) -> i64 {
    if a.is_nan() {
        return 0;
    }
    let floor = a.floor();
    let rounded = if a - floor >= 0.5 { floor + 1.0 } else { floor };
    rounded as i64
}

/// A class used to represent a set of many, potentially large, values (e.g.
/// many long strings such as URLs), using a significantly smaller amount of
/// memory.
///
/// The set is "lossy" in that it cannot definitively state that is does
/// contain a value but it *can* definitively say if a value is *not* in the
/// set. It can therefore be used as a Bloom Filter. Another application of
/// the set is that it can be used to perform fuzzy counting because it can
/// estimate reasonably accurately how many unique values are contained in
/// the set.
///
/// This class is NOT threadsafe.
///
/// Internally a Bitset is used to record values and once a client has
/// finished recording a stream of values the [`downsize`](Self::downsize)
/// method can be used to create a suitably smaller set that is sized
/// appropriately for the number of values recorded and desired saturation
/// levels.
///
/// Port of `org.apache.lucene.codecs.bloom.FuzzySet`.
#[derive(Debug, Clone)]
pub struct FuzzySet {
    filter: FixedBitSet,
    bloom_size: i32,
    hash_count: i32,
}

impl FuzzySet {
    /// Rounds down required `max_number_of_bits` to the nearest number that
    /// is made up of all ones as a binary number. Use this method where
    /// controlling memory use is paramount.
    ///
    /// Port of the deprecated `getNearestSetSize(int)`.
    #[deprecated(
        note = "public visibility of this method will be removed in a future release; this is \
                meant for internal use only"
    )]
    pub fn get_nearest_set_size(max_number_of_bits: i32) -> i32 {
        nearest_set_size(max_number_of_bits)
    }

    /// Use this method to choose a set size where accuracy (low content
    /// saturation) is more important than deciding how much memory to throw
    /// at the problem. `desired_saturation` is a number between 0 and 1
    /// expressing the % of bits set once all values have been recorded;
    /// returns the size of the set nearest to the required size, or -1.
    ///
    /// Port of the deprecated `getNearestSetSize(int, float)`.
    #[deprecated(note = "this method will be removed in a future release")]
    pub fn get_nearest_set_size_for_saturation(
        max_number_of_values_expected: i32,
        desired_saturation: f32,
    ) -> i32 {
        nearest_set_size_for_saturation(max_number_of_values_expected, desired_saturation)
    }

    /// Port of `createSetBasedOnMaxMemory(int)`.
    pub fn create_set_based_on_max_memory(max_num_bytes: i32) -> FuzzySet {
        let set_size = nearest_set_size(max_num_bytes.wrapping_mul(8));
        FuzzySet::new(new_bit_set(set_size), set_size, 1)
    }

    /// Port of the deprecated `createSetBasedOnQuality(int, float, int)`.
    #[deprecated(note = "this ctor will be removed in a future release; please see \
                `create_optimal_set` for an improved initialization approach")]
    pub fn create_set_based_on_quality(
        max_num_unique_values: i32,
        desired_max_saturation: f32,
        _version: i32,
    ) -> FuzzySet {
        let set_size =
            nearest_set_size_for_saturation(max_num_unique_values, desired_max_saturation);
        FuzzySet::new(new_bit_set(set_size), set_size, 1)
    }

    /// Port of `createOptimalSet(int, float)`.
    pub fn create_optimal_set(max_num_unique_values: i32, target_max_fpp: f32) -> FuzzySet {
        let set_size = ((f64::from(max_num_unique_values) * f64::from(target_max_fpp).ln())
            / (1.0 / 2f64.powf(2f64.ln())).ln())
        .ceil() as i32;
        let set_size = nearest_set_size(set_size.wrapping_mul(2));
        let optimal_k =
            java_math_round((f64::from(set_size) / f64::from(max_num_unique_values)) * 2f64.ln())
                as i32;
        FuzzySet::new(new_bit_set(set_size), set_size, optimal_k)
    }

    fn new(filter: FixedBitSet, bloom_size: i32, hash_count: i32) -> FuzzySet {
        FuzzySet {
            filter,
            bloom_size,
            hash_count,
        }
    }

    /// The main method required for a Bloom filter which, given a value
    /// determines set membership. Unlike a conventional set, the fuzzy set
    /// returns NO or MAYBE rather than true or false. Hash generation
    /// follows the same principles as [`add_value`](Self::add_value).
    ///
    /// Port of `contains(BytesRef)`.
    pub fn contains(&self, value: &BytesRef) -> ContainsResult {
        let hash = StringHelper::murmurhash3_x64_128_bytes_ref(value);

        let msb = hash[0];
        let lsb = hash[1];
        for i in 0..self.hash_count {
            let bloom_pos =
                (lsb.wrapping_add(i64::from(i).wrapping_mul(msb)) as i32) & self.bloom_size;
            if !self.may_contain_value(bloom_pos) {
                return ContainsResult::NO;
            }
        }
        ContainsResult::MAYBE
    }

    /// Serializes the data set to file using the following format:
    ///
    /// * FuzzySet --> hashCount, BloomSize, NumBitSetWords,
    ///   BitSetWord<sup>NumBitSetWords</sup>
    /// * hashCount --> `DataOutput.writeVInt` Uint32: the number of hash
    ///   functions (k).
    /// * BloomSize --> `DataOutput.writeInt` Uint32: the modulo value used to
    ///   project hashes into the field's Bitset
    /// * NumBitSetWords --> `DataOutput.writeInt` Uint32: the number of longs
    ///   (as returned from `FixedBitSet.getBits`)
    /// * BitSetWord --> `DataOutput.writeLong` Long: a long from the array
    ///   returned by `FixedBitSet.getBits`
    ///
    /// Port of `serialize(DataOutput)`.
    ///
    /// # Errors
    ///
    /// If there is a low-level I/O error.
    pub fn serialize(&self, out: &mut dyn DataOutput) -> Result<()> {
        out.write_v_int(self.hash_count)?;
        out.write_int(self.bloom_size)?;
        let bits = self.filter.get_bits();
        out.write_int(bits.len() as i32)?;
        for word in bits {
            // Can't used VLong encoding because cant cope with negative numbers
            // output by FixedBitSet
            out.write_long(word.load(Ordering::Relaxed) as i64)?;
        }
        Ok(())
    }

    /// Port of `deserialize(DataInput)`.
    ///
    /// # Errors
    ///
    /// I/O errors; `NegativeArraySizeException` for a negative word count
    /// and `IllegalArgumentException` when the words cannot hold `bloomSize
    /// + 1` bits (`new FixedBitSet(long[], int)`).
    pub fn deserialize(input: &mut dyn DataInput) -> Result<FuzzySet> {
        let hash_count = input.read_v_int()?;
        let bloom_size = input.read_int()?;
        let num_longs = input.read_int()?;
        if num_longs < 0 {
            return Err(LuceneError::unchecked(
                "java.lang.NegativeArraySizeException",
                num_longs.to_string(),
            ));
        }
        let mut longs = vec![0i64; num_longs as usize];
        input.read_longs(Some(&mut longs), 0, num_longs as usize)?;
        let bits = FixedBitSet::from_bits(
            longs.into_iter().map(|l| l as u64).collect(),
            num_bits(bloom_size),
        )?;
        Ok(FuzzySet::new(bits, bloom_size, hash_count))
    }

    fn may_contain_value(&self, a_hash: i32) -> bool {
        // Bloom sizes are always base 2 and so can be ANDed for a fast modulo
        let pos = a_hash & self.bloom_size;
        self.filter.get(pos as usize)
    }

    /// Records a value in the set. The referenced bytes are hashed. From the
    /// 64-bit generated hash, two 32-bit hashes are derived from the msb and
    /// lsb which can be used to derive more hashes (see
    /// <https://www.eecs.harvard.edu/~michaelm/postscripts/rsa2008.pdf>).
    /// Finally, each generated hash is modulo n'd where n is the chosen size
    /// of the internal bitset.
    ///
    /// Port of `addValue(BytesRef)`.
    pub fn add_value(&mut self, value: &BytesRef) {
        let hash = StringHelper::murmurhash3_x64_128_bytes_ref(value);
        let msb = hash[0];
        let lsb = hash[1];
        for i in 0..self.hash_count {
            // Bitmasking using bloomSize is effectively a modulo operation.
            let bloom_pos =
                (lsb.wrapping_add(i64::from(i).wrapping_mul(msb)) as i32) & self.bloom_size;
            self.filter.set(bloom_pos as usize);
        }
    }

    /// `target_max_saturation` is a number between 0 and 1 describing the %
    /// of bits that would ideally be set in the result. Lower values have
    /// better accuracy but require more space. Returns a smaller `FuzzySet`
    /// or `None` if the current set is already over-saturated.
    ///
    /// Port of `downsize(float)`.
    pub fn downsize(&self, target_max_saturation: f32) -> Option<FuzzySet> {
        let num_bits_set = self.filter.cardinality() as i32;
        let mut right_sized_bit_set_size = self.bloom_size;
        // Hopefully find a smaller size bitset into which we can project accumulated values while
        // maintaining desired saturation level
        for &candidate_bitset_size in &USABLE_BIT_SET_SIZES {
            let candidate_saturation = num_bits_set as f32 / candidate_bitset_size as f32;
            if candidate_saturation <= target_max_saturation {
                right_sized_bit_set_size = candidate_bitset_size;
                break;
            }
        }
        // Re-project the numbers to a smaller space if necessary
        if right_sized_bit_set_size < self.bloom_size {
            // Reset the choice of bitset to the smaller version
            let mut right_sized_bit_set = new_bit_set(right_sized_bit_set_size);
            // Map across the bits from the large set to the smaller one
            let mut bit_index = 0i32;
            loop {
                bit_index = self.filter.next_set_bit(bit_index);
                if bit_index != NO_MORE_DOCS {
                    // Project the larger number into a smaller one effectively
                    // modulo-ing by using the target bitset size as a mask
                    let down_sized_bit_index = bit_index & right_sized_bit_set_size;
                    right_sized_bit_set.set(down_sized_bit_index as usize);
                    bit_index += 1;
                }
                if !(bit_index >= 0 && bit_index <= self.bloom_size) {
                    break;
                }
            }
            Some(FuzzySet::new(
                right_sized_bit_set,
                right_sized_bit_set_size,
                self.hash_count,
            ))
        } else {
            None
        }
    }

    /// Port of `getEstimatedUniqueValues()`.
    pub fn get_estimated_unique_values(&self) -> i32 {
        estimated_number_unique_values_allowing_for_collisions(
            self.bloom_size,
            self.filter.cardinality() as i32,
            self.hash_count,
        )
    }

    /// Given a set size and the number of set bits, produces an estimate of
    /// the number of unique values recorded (assuming a single hash function
    /// is used).
    ///
    /// Port of the deprecated
    /// `getEstimatedNumberUniqueValuesAllowingForCollisions(int, int)`.
    #[deprecated(note = "this method will be removed in a future release; see \
                `get_estimated_unique_values` for an alternative based on an actual `FuzzySet` \
                instance")]
    pub fn get_estimated_number_unique_values_allowing_for_collisions(
        set_size: i32,
        num_recorded_bits: i32,
    ) -> i32 {
        estimated_number_unique_values_allowing_for_collisions(set_size, num_recorded_bits, 1)
    }

    /// Given a set size, the number of set bits and hash function count,
    /// produces an estimate of the number of unique values recorded.
    ///
    /// Port of the deprecated
    /// `getEstimatedNumberUniqueValuesAllowingForCollisions(int, int, int)`.
    #[deprecated(note = "this method will be removed in a future release; see \
                `get_estimated_unique_values` for an alternative based on an actual `FuzzySet` \
                instance")]
    pub fn get_estimated_number_unique_values_allowing_for_collisions_with_hash_count(
        set_size: i32,
        num_recorded_bits: i32,
        hash_count: i32,
    ) -> i32 {
        estimated_number_unique_values_allowing_for_collisions(
            set_size,
            num_recorded_bits,
            hash_count,
        )
    }

    /// Port of `getTargetMaxSaturation()`.
    pub fn get_target_max_saturation(&self) -> f32 {
        0.5
    }

    /// Port of `getSaturation()`.
    pub fn get_saturation(&self) -> f32 {
        let num_bits_set = self.filter.cardinality() as i32;
        num_bits_set as f32 / self.bloom_size as f32
    }
}

/// `new FixedBitSet(setSize + 1)`.
fn new_bit_set(set_size: i32) -> FixedBitSet {
    FixedBitSet::new(num_bits(set_size))
}

/// `setSize + 1` as a bit count; Java's `FixedBitSet` treats a negative
/// count as zero words.
fn num_bits(set_size: i32) -> usize {
    usize::try_from(i64::from(set_size) + 1).unwrap_or(0)
}

/// The body of `getNearestSetSize(int)`.
fn nearest_set_size(max_number_of_bits: i32) -> i32 {
    let mut result = USABLE_BIT_SET_SIZES[0];
    for &size in &USABLE_BIT_SET_SIZES {
        if size <= max_number_of_bits {
            result = size;
        }
    }
    result
}

/// The body of `getNearestSetSize(int, float)`.
fn nearest_set_size_for_saturation(
    max_number_of_values_expected: i32,
    desired_saturation: f32,
) -> i32 {
    // Iterate around the various scales of bitset from smallest to largest looking for the first
    // that
    // satisfies value volumes at the chosen saturation level
    for &size in &USABLE_BIT_SET_SIZES {
        let num_set_bits_at_desired_saturation = (size as f32 * desired_saturation) as i32;
        let estimated_num_unique_values = estimated_number_unique_values_allowing_for_collisions(
            size,
            num_set_bits_at_desired_saturation,
            1,
        );
        if estimated_num_unique_values > max_number_of_values_expected {
            return size;
        }
    }
    -1
}

/// The body of `getEstimatedNumberUniqueValuesAllowingForCollisions(int,
/// int, int)`.
fn estimated_number_unique_values_allowing_for_collisions(
    set_size: i32,
    num_recorded_bits: i32,
    hash_count: i32,
) -> i32 {
    let set_size_as_double = f64::from(set_size);
    let num_recorded_bits_as_double = f64::from(num_recorded_bits);
    let hash_count_as_double = f64::from(hash_count);
    let saturation = num_recorded_bits_as_double / set_size_as_double;
    // `Math.log(1 - saturation) * -1`: negation is exact.
    let log_inverse_saturation = -(1.0 - saturation).ln();
    (set_size_as_double * log_inverse_saturation / hash_count_as_double) as i32
}

impl Accountable for FuzzySet {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::shallow_size_of_array::<i64>(self.filter.get_bits().len())
    }
}

impl fmt::Display for FuzzySet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "FuzzySet(k={}, bits={}/{})",
            self.hash_count,
            self.filter.cardinality(),
            self.filter.length()
        )
    }
}
