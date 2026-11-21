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

//! Port of `org.apache.lucene.codecs.uniformsplit.BlockDecoder`.

use std::fmt;

use rucene::error::Result;
use rucene::store::DataInput;
use rucene::util::BytesRef;

/// Decodes the raw bytes of a block when the index is read, according to the
/// [`BlockEncoder`](super::BlockEncoder) used during the writing of the index.
///
/// For example, implementations may decompress or decrypt.
///
/// Port of the interface `org.apache.lucene.codecs.uniformsplit.BlockDecoder`
/// (`@lucene.experimental`). [`fmt::Display`] stands for the `toString()`
/// that `UniformSplitPostingsFormat`'s argument validation concatenates.
pub trait BlockDecoder: Send + Sync + fmt::Display {
    /// Decodes all the bytes of one block in a single operation. The decoding
    /// is per block.
    ///
    /// `block_bytes` is the input block bytes to read; `length` the number of
    /// bytes to read from the input. Returns the decoded block bytes.
    ///
    /// Port of `BlockDecoder.decode(DataInput, long)`.
    ///
    /// # Errors
    ///
    /// If a decoding error occurs.
    // TODO: this method could return a new interface ReadableBytes which
    // would provide the size and create PositionableDataInput (get/set position)
    // implemented by ByteArrayDataInput and ByteBuffersDataInput.
    // Big length could be supported (> Integer.MAX_VALUE).
    fn decode(&self, block_bytes: &mut dyn DataInput, length: i64) -> Result<BytesRef>;
}
