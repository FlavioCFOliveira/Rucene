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
//! `RamUsageEstimator.shallowSizeOfInstance`; this port measures the shallow
//! sizes of the Rust types that stand for them (`BytesRef`, `Vec<u8>`,
//! `HashMap`, a key-value pair and `Vec`), the convention of the crate's
//! other `ramBytesUsed` ports.

use std::collections::HashMap;

use rucene::codecs::BlockTermState;
use rucene::util::{BytesRef, RamUsageEstimator};

use super::delta_base_term_state_serializer::DeltaBaseTermStateSerializer;

/// Port of `RamUsageUtil.ramBytesUsed(BytesRef)`.
pub fn ram_bytes_used_bytes_ref(bytes_ref: &BytesRef) -> i64 {
    RamUsageEstimator::shallow_size_of_instance::<BytesRef>()
        + RamUsageEstimator::size_of(&bytes_ref.bytes)
}

/// Port of `RamUsageUtil.ramBytesUsed(BytesRefBuilder)`, for the builder's
/// bytes.
pub fn ram_bytes_used_bytes_ref_builder(bytes: &[u8]) -> i64 {
    RamUsageEstimator::shallow_size_of_instance::<Vec<u8>>()
        + RamUsageEstimator::shallow_size_of_instance::<BytesRef>()
        + RamUsageEstimator::size_of(bytes)
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
    RamUsageEstimator::shallow_size_of_instance::<HashMap<(), ()>>()
        + ram_bytes_used_by_object_array_of_length((f64::from(size) / 0.6) as i32)
        + RamUsageEstimator::shallow_size_of_instance::<(usize, usize)>() * i64::from(size)
}

/// Port of `RamUsageUtil.ramBytesUsedByUnmodifiableArrayListOfSize(int)`.
pub fn ram_bytes_used_by_unmodifiable_array_list_of_size(size: i32) -> i64 {
    RamUsageEstimator::shallow_size_of_instance::<Vec<()>>()
        + ram_bytes_used_by_object_array_of_length(size)
}

/// Port of `RamUsageUtil.ramBytesUsedByObjectArrayOfLength(int)`.
pub fn ram_bytes_used_by_object_array_of_length(length: i32) -> i64 {
    RamUsageEstimator::align_object_size(
        RamUsageEstimator::NUM_BYTES_ARRAY_HEADER
            + RamUsageEstimator::NUM_BYTES_OBJECT_REF * i64::from(length),
    )
}
