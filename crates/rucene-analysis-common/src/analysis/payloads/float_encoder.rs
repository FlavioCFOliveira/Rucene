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

//! Encodes a character array float as payload bytes, ported from
//! `org.apache.lucene.analysis.payloads.FloatEncoder`.

use rucene::error::Result;
use rucene::util::BytesRef;

use super::abstract_encoder::AbstractEncoder;
use super::payload_helper::PayloadHelper;
use crate::analysis::synonym::word2vec::dl4j_model_reader::parse_float;

/// Encode a character array Float as a `BytesRef`.
///
/// See `PayloadHelper#encodeFloat(float, byte[], int)`.
///
/// Equivalent to `org.apache.lucene.analysis.payloads.FloatEncoder`.
#[derive(Debug, Clone, Copy, Default)]
pub struct FloatEncoder;

impl FloatEncoder {
    /// Equivalent to the implicit `FloatEncoder()`.
    pub fn new() -> Self {
        Self
    }
}

impl AbstractEncoder for FloatEncoder {
    /// Equivalent to `FloatEncoder.encode(char[], int, int)`:
    /// `Float.parseFloat(new String(buffer, offset, length))`, written as
    /// `PayloadHelper.encodeFloat` writes it. An unpaired surrogate of the
    /// units becomes U+FFFD in the string, which `parseFloat` refuses as
    /// it refuses the surrogate.
    ///
    /// # Errors
    ///
    /// The `NumberFormatException` of `Float.parseFloat`.
    fn encode_range(&self, buffer: &[u16], offset: usize, length: usize) -> Result<BytesRef> {
        // TODO: improve this so that we don't have to new Strings
        let payload = parse_float(&String::from_utf16_lossy(&buffer[offset..offset + length]))?;
        let bytes = PayloadHelper::encode_float(payload);
        Ok(BytesRef::new(bytes))
    }
}
