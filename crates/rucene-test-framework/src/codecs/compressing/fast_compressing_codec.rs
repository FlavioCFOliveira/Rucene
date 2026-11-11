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

//! Port of `org.apache.lucene.tests.codecs.compressing.FastCompressingCodec`.

use std::sync::Arc;

use rucene::codecs::compressing;
use rucene::error::Result;

use super::CompressingCodec;

/// A compressing codec over `CompressionMode.FAST`.
///
/// Port of `FastCompressingCodec` (`extends CompressingCodec`, named `FastCompressingStoredFieldsData`).
pub struct FastCompressingCodec;

impl FastCompressingCodec {
    /// `new FastCompressingCodec(chunkSize, maxDocsPerChunk, withSegmentSuffix, blockSize)`.
    ///
    /// # Errors
    ///
    /// As [`CompressingCodec::new`].
    #[allow(clippy::new_ret_no_self)]
    pub fn new(
        chunk_size: i32,
        max_docs_per_chunk: i32,
        with_segment_suffix: bool,
        block_size: i32,
    ) -> Result<CompressingCodec> {
        CompressingCodec::new(
            "FastCompressingStoredFieldsData",
            if with_segment_suffix {
                "FastCompressingStoredFields"
            } else {
                ""
            },
            Arc::clone(&compressing::FAST),
            chunk_size,
            max_docs_per_chunk,
            block_size,
        )
    }

    /// `new FastCompressingCodec()`: `this(1 << 14, 128, false, 10)`.
    ///
    /// # Errors
    ///
    /// As [`CompressingCodec::new`].
    pub fn default_instance() -> Result<CompressingCodec> {
        Self::new(1 << 14, 128, false, 10)
    }
}
