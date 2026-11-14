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

//! Encodes characters as payload bytes in a charset, ported from
//! `org.apache.lucene.analysis.payloads.IdentityEncoder`.

use rucene::error::Result;
use rucene::util::BytesRef;

use super::abstract_encoder::AbstractEncoder;
use crate::jdk::charset::StandardCharset;

/// Does nothing other than convert the char array to a byte array using the
/// specified encoding.
///
/// Equivalent to `org.apache.lucene.analysis.payloads.IdentityEncoder`. Its
/// protected `Charset charset` is a [`StandardCharset`], the charsets the
/// port encodes as Java's `Charset.encode` does (see `jdk::charset`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityEncoder {
    /// The protected `charset` field, `StandardCharsets.UTF_8` by default.
    pub charset: StandardCharset,
}

impl IdentityEncoder {
    /// Equivalent to `IdentityEncoder()`: UTF-8.
    pub fn new() -> Self {
        Self {
            charset: StandardCharset::Utf8,
        }
    }

    /// Equivalent to `IdentityEncoder(Charset)`.
    pub fn with_charset(charset: StandardCharset) -> Self {
        Self { charset }
    }
}

impl Default for IdentityEncoder {
    fn default() -> Self {
        Self::new()
    }
}

impl AbstractEncoder for IdentityEncoder {
    /// Equivalent to `IdentityEncoder.encode(char[], int, int)`:
    /// `charset.encode(CharBuffer.wrap(buffer, offset, length))`, whose
    /// malformed and unmappable input is replaced.
    fn encode_range(&self, buffer: &[u16], offset: usize, length: usize) -> Result<BytesRef> {
        Ok(BytesRef::new(
            self.charset.encode(&buffer[offset..offset + length]),
        ))
    }
}
