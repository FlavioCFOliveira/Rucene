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

//! Converts char buffers to payload bytes, ported from
//! `org.apache.lucene.analysis.payloads.PayloadEncoder`.

use std::fmt::Debug;

use rucene::error::Result;
use rucene::util::BytesRef;

/// Mainly for use with the DelimitedPayloadTokenFilter, converts char
/// buffers to `BytesRef`.
///
/// NOTE: This interface is subject to change
///
/// Equivalent to `org.apache.lucene.analysis.payloads.PayloadEncoder`. The
/// buffers are Java's `char[]`, UTF-16 units.
pub trait PayloadEncoder: Debug + Send + Sync {
    /// Equivalent to `PayloadEncoder.encode(char[])`.
    ///
    /// # Errors
    ///
    /// What the encoding throws (a `NumberFormatException` for a number
    /// encoder).
    fn encode(&self, buffer: &[u16]) -> Result<BytesRef>;

    /// Convert a char array to a `BytesRef`
    ///
    /// Returns the encoded `BytesRef`.
    ///
    /// Equivalent to `PayloadEncoder.encode(char[], int, int)`.
    ///
    /// # Errors
    ///
    /// As [`encode`](Self::encode).
    fn encode_range(&self, buffer: &[u16], offset: usize, length: usize) -> Result<BytesRef>;
}
