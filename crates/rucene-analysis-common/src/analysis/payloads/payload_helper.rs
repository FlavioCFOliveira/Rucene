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

//! Utility methods for encoding payloads, ported from
//! `org.apache.lucene.analysis.payloads.PayloadHelper`.

/// Utility methods for encoding payloads.
///
/// Equivalent to `org.apache.lucene.analysis.payloads.PayloadHelper`, a
/// class of static methods: its associated functions. Values are written
/// big-endian, as `BitUtil.VH_BE_FLOAT` and `VH_BE_INT` write them, a float
/// by its raw bits.
#[derive(Debug, Clone, Copy, Default)]
pub struct PayloadHelper;

impl PayloadHelper {
    /// Equivalent to `PayloadHelper.encodeFloat(float)`.
    pub fn encode_float(payload: f32) -> Vec<u8> {
        let mut data = vec![0u8; 4];
        Self::encode_float_into(payload, &mut data, 0);
        data
    }

    /// Equivalent to `PayloadHelper.encodeFloat(float, byte[], int)`,
    /// returning `data`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IndexOutOfBoundsException`: fewer than four bytes
    /// at `offset`.
    pub fn encode_float_into(payload: f32, data: &mut [u8], offset: usize) -> &mut [u8] {
        data[offset..offset + 4].copy_from_slice(&payload.to_bits().to_be_bytes());
        data
    }

    /// Equivalent to `PayloadHelper.encodeInt(int)`.
    pub fn encode_int(payload: i32) -> Vec<u8> {
        let mut data = vec![0u8; 4];
        Self::encode_int_into(payload, &mut data, 0);
        data
    }

    /// Equivalent to `PayloadHelper.encodeInt(int, byte[], int)`, returning
    /// `data`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IndexOutOfBoundsException`: fewer than four bytes
    /// at `offset`.
    pub fn encode_int_into(payload: i32, data: &mut [u8], offset: usize) -> &mut [u8] {
        data[offset..offset + 4].copy_from_slice(&payload.to_be_bytes());
        data
    }

    /// the decoded float
    ///
    /// Equivalent to `PayloadHelper.decodeFloat(byte[])`.
    ///
    /// # Panics
    ///
    /// As [`decode_float_at`](Self::decode_float_at).
    pub fn decode_float(bytes: &[u8]) -> f32 {
        Self::decode_float_at(bytes, 0)
    }

    /// Decode the payload that was encoded using
    /// [`encode_float`](Self::encode_float). NOTE: the length of the array
    /// must be at least offset + 4 long.
    ///
    /// * `bytes` The bytes to decode
    /// * `offset` The offset into the array.
    ///
    /// Returns the float that was encoded.
    ///
    /// Equivalent to `PayloadHelper.decodeFloat(byte[], int)`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IndexOutOfBoundsException`: fewer than four bytes
    /// at `offset`.
    pub fn decode_float_at(bytes: &[u8], offset: usize) -> f32 {
        f32::from_bits(u32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("INVARIANT: four bytes"),
        ))
    }

    /// Equivalent to `PayloadHelper.decodeInt(byte[], int)`.
    ///
    /// # Panics
    ///
    /// Where Java throws `IndexOutOfBoundsException`: fewer than four bytes
    /// at `offset`.
    pub fn decode_int(bytes: &[u8], offset: usize) -> i32 {
        i32::from_be_bytes(
            bytes[offset..offset + 4]
                .try_into()
                .expect("INVARIANT: four bytes"),
        )
    }
}
