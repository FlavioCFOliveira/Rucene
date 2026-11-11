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

//! Port of `org.apache.lucene.tests.codecs.compressing.dummy.DummyCompressingCodec`.

use std::sync::{Arc, LazyLock};

use rucene::codecs::{CompressionMode, Compressor, Decompressor};
use rucene::error::Result;
use rucene::store::{ByteBuffersDataInput, DataInput, DataOutput};
use rucene::util::{ArrayUtil, BytesRef};

use crate::codecs::compressing::CompressingCodec;

/// `DummyCompressingCodec.DUMMY`: a `CompressionMode` that does not compress.
pub static DUMMY: LazyLock<Arc<dyn CompressionMode>> = LazyLock::new(|| Arc::new(DummyMode));

/// The anonymous `CompressionMode` of `DUMMY`.
#[derive(Debug)]
struct DummyMode;

impl CompressionMode for DummyMode {
    fn new_compressor(&self) -> Box<dyn Compressor> {
        Box::new(DummyCompressor)
    }

    fn new_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(DummyDecompressor)
    }

    fn to_string(&self) -> String {
        "DUMMY".to_string()
    }
}

/// `DUMMY_DECOMPRESSOR`: reads the requested bytes as they are.
#[derive(Debug, Clone, Copy)]
struct DummyDecompressor;

impl Decompressor for DummyDecompressor {
    fn decompress(
        &mut self,
        input: &mut dyn DataInput,
        original_length: usize,
        offset: usize,
        length: usize,
        bytes: &mut BytesRef,
    ) -> Result<()> {
        if cfg!(debug_assertions) && offset + length > original_length {
            return Err(rucene::error::LuceneError::AssertionError(String::new()));
        }
        if bytes.bytes.len() < original_length {
            bytes.bytes = vec![0; ArrayUtil::oversize(original_length, 1)];
        }
        input.read_bytes(Some(&mut bytes.bytes), 0, offset + length)?;
        bytes.offset = offset;
        bytes.length = length;
        Ok(())
    }

    /// Java's `clone()` returns `this`; this returns an equal copy of the unit
    /// struct.
    fn clone_decompressor(&self) -> Box<dyn Decompressor> {
        Box::new(*self)
    }
}

/// `DUMMY_COMPRESSOR`: copies its input.
#[derive(Debug)]
struct DummyCompressor;

impl Compressor for DummyCompressor {
    fn compress(
        &mut self,
        input: &mut ByteBuffersDataInput,
        out: &mut dyn DataOutput,
    ) -> Result<()> {
        // `out.copyBytes(buffersInput, buffersInput.length())`
        // (`DummyCompressingCodec.java:80-82`).
        let len = input.length() as i64;
        out.copy_bytes(input, len)
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// CompressionCodec that does not compress data, useful for testing.
///
/// Port of `DummyCompressingCodec` (`extends CompressingCodec`, named
/// `DummyCompressingStoredFieldsData`).
pub struct DummyCompressingCodec;

impl DummyCompressingCodec {
    /// `new DummyCompressingCodec(chunkSize, maxDocsPerChunk,
    /// withSegmentSuffix, blockSize)`: allows to configure the chunk size.
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
            "DummyCompressingStoredFieldsData",
            if with_segment_suffix {
                "DummyCompressingStoredFields"
            } else {
                ""
            },
            Arc::clone(&DUMMY),
            chunk_size,
            max_docs_per_chunk,
            block_size,
        )
    }

    /// `new DummyCompressingCodec()`: `this(1 << 14, 128, false, 10)`.
    ///
    /// # Errors
    ///
    /// As [`CompressingCodec::new`].
    pub fn default_instance() -> Result<CompressingCodec> {
        Self::new(1 << 14, 128, false, 10)
    }
}
