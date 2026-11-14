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

//! Base class for payload encoders, ported from
//! `org.apache.lucene.analysis.payloads.AbstractEncoder`.

use std::fmt::Debug;

use rucene::error::Result;
use rucene::util::BytesRef;

use super::payload_encoder::PayloadEncoder;

/// Base class for payload encoders.
///
/// Equivalent to `org.apache.lucene.analysis.payloads.AbstractEncoder`, an
/// abstract class that implements `PayloadEncoder.encode(char[])` as
/// `encode(buffer, 0, buffer.length)`: an encoder implements this trait's
/// [`encode_range`](AbstractEncoder::encode_range), and is a
/// [`PayloadEncoder`] through the blanket implementation that supplies that
/// `encode`.
pub trait AbstractEncoder: Debug + Send + Sync {
    /// The subclass's `encode(char[], int, int)`.
    ///
    /// # Errors
    ///
    /// What the encoding throws.
    fn encode_range(&self, buffer: &[u16], offset: usize, length: usize) -> Result<BytesRef>;
}

impl<T: AbstractEncoder> PayloadEncoder for T {
    /// Equivalent to `AbstractEncoder.encode(char[])`, `encode(buffer, 0,
    /// buffer.length)`.
    fn encode(&self, buffer: &[u16]) -> Result<BytesRef> {
        AbstractEncoder::encode_range(self, buffer, 0, buffer.len())
    }

    fn encode_range(&self, buffer: &[u16], offset: usize, length: usize) -> Result<BytesRef> {
        AbstractEncoder::encode_range(self, buffer, offset, length)
    }
}
