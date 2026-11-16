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

//! Port of `org.apache.lucene.codecs.uniformsplit.RamUsageUtil`.
//!
//! Utility methods to estimate the RAM usage of objects. It relies on
//! [`RamUsageEstimator`] (`@lucene.experimental`).
//!
//! Java measures the shallow sizes of `BytesRef`, `BytesRefBuilder`,
//! `HashMap`, a `HashMap` entry and an unmodifiable `ArrayList` once, with
//! `RamUsageEstimator.shallowSizeOfInstance`/`shallowSizeOf`; the port
//! computes the same sizes from the Java classes' instance fields
//! ([`RamUsageEstimator::shallow_size_of_declared_instance`]): 24, 16, 48,
//! 32 and 24 + 24 bytes on JDK 25 (64-bit, compressed oops), as a Lucene
//! 10.5.0 probe reports.

use rucene::codecs::BlockTermState;
use rucene::util::{BytesRef, RamUsageEstimator};

use super::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;

/// Port of `RamUsageUtil.ramBytesUsed(BytesRef)`.
pub fn ram_bytes_used_bytes_ref(bytes_ref: &BytesRef) -> i64 {
    bytes_ref_base_ram_usage() + RamUsageEstimator::size_of(&bytes_ref.bytes)
}

const REF: i64 = RamUsageEstimator::NUM_BYTES_OBJECT_REF;

/// `BYTES_REF_BASE_RAM_USAGE`: `BytesRef`'s `byte[] bytes`, `int offset`,
/// `int length`.
fn bytes_ref_base_ram_usage() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&[REF, 4, 4])
}

/// `BYTES_REF_BUILDER_BASE_RAM_USAGE`: `BytesRefBuilder`'s `BytesRef ref`.
fn bytes_ref_builder_base_ram_usage() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&[REF])
}

/// `HASH_MAP_BASE_RAM_USAGE`: `HashMap`'s fields (`AbstractMap.keySet`,
/// `AbstractMap.values`, `table`, `entrySet`, `size`, `modCount`,
/// `threshold`, `loadFactor`).
fn hash_map_base_ram_usage() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&[REF, REF, REF, REF, 4, 4, 4, 4])
}

/// `HASH_MAP_ENTRY_BASE_RAM_USAGE`: a `HashMap.Node` (`hash`, `key`,
/// `value`, `next`).
fn hash_map_entry_base_ram_usage() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&[4, REF, REF, REF])
}

/// `UNMODIFIABLE_ARRAY_LIST_BASE_RAM_USAGE`: an
/// `UnmodifiableRandomAccessList` (`c`, `list`) plus an `ArrayList`
/// (`AbstractList.modCount`, `elementData`, `size`).
fn unmodifiable_array_list_base_ram_usage() -> i64 {
    RamUsageEstimator::shallow_size_of_declared_instance(&[REF, REF])
        + RamUsageEstimator::shallow_size_of_declared_instance(&[4, REF, 4])
}

/// Port of `RamUsageUtil.ramBytesUsed(BytesRefBuilder)`, for a builder
/// whose byte array holds `capacity` bytes.
pub fn ram_bytes_used_bytes_ref_builder(capacity: usize) -> i64 {
    bytes_ref_builder_base_ram_usage()
        + bytes_ref_base_ram_usage()
        + RamUsageEstimator::shallow_size_of_array::<u8>(capacity)
}

/// Port of `RamUsageUtil.ramBytesUsed(TermState)`.
pub fn ram_bytes_used_term_state(term_state: &BlockTermState) -> i64 {
    DeltaBaseTermStateSerializer::ram_bytes_used_of(term_state)
}

/// Port of `RamUsageUtil.ramBytesUsedByByteArrayOfLength(int)`.
pub fn ram_bytes_used_by_byte_array_of_length(length: i32) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + i64::from(length),
    )
}

/// Port of `RamUsageUtil.ramBytesUsedByHashMapOfSize(int)`.
pub fn ram_bytes_used_by_hash_map_of_size(size: i32) -> i64 {
    hash_map_base_ram_usage()
        + ram_bytes_used_by_object_array_of_length((f64::from(size) / 0.6) as i32)
        + hash_map_entry_base_ram_usage() * i64::from(size)
}

/// Port of `RamUsageUtil.ramBytesUsedByUnmodifiableArrayListOfSize(int)`.
pub fn ram_bytes_used_by_unmodifiable_array_list_of_size(size: i32) -> i64 {
    unmodifiable_array_list_base_ram_usage() + ram_bytes_used_by_object_array_of_length(size)
}

/// Port of `RamUsageUtil.ramBytesUsedByObjectArrayOfLength(int)`.
pub fn ram_bytes_used_by_object_array_of_length(length: i32) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF * i64::from(length),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression (rmp #528): the base sizes are the Java classes' (JDK 25
    /// probe of Lucene 10.5.0: `BytesRef` 24, `BytesRefBuilder` 16,
    /// `HashMap` 48, a `HashMap` entry 32, an unmodifiable `ArrayList`
    /// 24 + 24), not the Rust types'.
    #[test]
    fn base_sizes_match_lucene() {
        assert_eq!(
            ram_bytes_used_bytes_ref(&BytesRef::new(vec![0u8; 64])),
            24 + 80
        );
        assert_eq!(ram_bytes_used_bytes_ref_builder(0), 16 + 24 + 16);
        assert_eq!(ram_bytes_used_by_hash_map_of_size(1), 48 + 24 + 32);
        assert_eq!(
            ram_bytes_used_by_unmodifiable_array_list_of_size(0),
            48 + 16
        );
    }
}
