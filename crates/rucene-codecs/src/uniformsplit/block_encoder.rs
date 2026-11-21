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

//! Port of `org.apache.lucene.codecs.uniformsplit.BlockEncoder`.

use std::fmt;

use rucene::error::Result;
use rucene::store::{DataInput, DataOutput};

/// Encodes the raw bytes of a block when the index is written.
///
/// For example, implementations may compress or encrypt.
///
/// Port of the interface `org.apache.lucene.codecs.uniformsplit.BlockEncoder`
/// (`@lucene.experimental`). [`fmt::Display`] stands for the `toString()`
/// that `UniformSplitPostingsFormat`'s argument validation concatenates.
pub trait BlockEncoder: Send + Sync + fmt::Display {
    /// Encodes all the bytes of one block in a single operation. The encoding
    /// is per block.
    ///
    /// `block_bytes` is the input block bytes to read; `length` the number of
    /// bytes to read from the input. Returns the encoded block bytes.
    ///
    /// Port of `BlockEncoder.encode(DataInput, long)`.
    ///
    /// # Errors
    ///
    /// If an encoding error occurs.
    fn encode(
        &self,
        block_bytes: &mut dyn DataInput,
        length: i64,
    ) -> Result<Box<dyn WritableBytes>>;
}

/// Writable byte buffer.
///
/// Port of the interface `BlockEncoder.WritableBytes`.
pub trait WritableBytes {
    /// Gets the number of bytes.
    ///
    /// Port of `WritableBytes.size()`.
    fn size(&self) -> i64;

    /// Writes the bytes to the provided [`DataOutput`].
    ///
    /// Port of `WritableBytes.writeTo(DataOutput)`.
    ///
    /// # Errors
    ///
    /// The I/O errors of the output.
    fn write_to(&self, data_output: &mut dyn DataOutput) -> Result<()>;
}
