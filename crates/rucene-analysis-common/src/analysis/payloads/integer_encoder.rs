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

//! Encodes a character array integer as payload bytes, ported from
//! `org.apache.lucene.analysis.payloads.IntegerEncoder`.

use rucene::error::Result;
use rucene::util::{ArrayUtil, BytesRef};

use super::abstract_encoder::AbstractEncoder;
use super::payload_helper::PayloadHelper;

/// Encode a character array Integer as a `BytesRef`.
///
/// See `PayloadHelper#encodeInt(int, byte[], int)`.
///
/// Equivalent to `org.apache.lucene.analysis.payloads.IntegerEncoder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct IntegerEncoder;

impl IntegerEncoder {
    /// Equivalent to the implicit `IntegerEncoder()`.
    pub fn new() -> Self {
        Self
    }
}

impl AbstractEncoder for IntegerEncoder {
    /// Equivalent to `IntegerEncoder.encode(char[], int, int)`:
    /// `ArrayUtil.parseInt(buffer, offset, length)`, written as
    /// `PayloadHelper.encodeInt` writes it. `ArrayUtil.parseInt` reads each
    /// UTF-16 unit with `Character.digit(char, int)`; a surrogate unit,
    /// never a digit, is read as U+FFFD, which is not one either.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `ArrayUtil.parseInt`.
    fn encode_range(&self, buffer: &[u16], offset: usize, length: usize) -> Result<BytesRef> {
        // TODO: improve this so that we don't have to new Strings
        let units: Vec<char> = buffer[offset..offset + length]
            .iter()
            .map(|&unit| char::from_u32(u32::from(unit)).unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        let payload = ArrayUtil::parse_int(&units, 0, units.len())?;
        let bytes = PayloadHelper::encode_int(payload);
        Ok(BytesRef::new(bytes))
    }
}
