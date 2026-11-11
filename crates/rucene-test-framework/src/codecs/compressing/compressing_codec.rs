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

//! Port of `org.apache.lucene.tests.codecs.compressing.CompressingCodec`.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use rucene::codecs::lucene90::stored_fields::Lucene90CompressingStoredFieldsFormat;
use rucene::codecs::lucene90::term_vectors::Lucene90CompressingTermVectorsFormat;
use rucene::codecs::{
    Codec, CompoundFormat, CompressionMode, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat,
    LiveDocsFormat, NormsFormat, PointsFormat, PostingsFormat, SegmentInfoFormat,
    StoredFieldsFormat, TermVectorsFormat,
};
use rucene::error::Result;
use rucene::util::packed::DirectMonotonicWriter;

use super::dummy::DummyCompressingCodec;
use super::{
    DeflateWithPresetCompressingCodec, FastCompressingCodec, FastDecompressionCompressingCodec,
    HighCompressionCompressingCodec, LZ4WithPresetCompressingCodec,
};
use crate::util::TestUtil;

/// A codec that uses `Lucene90CompressingStoredFieldsFormat` for its stored
/// fields (and `Lucene90CompressingTermVectorsFormat` for its term vectors)
/// and delegates to the default codec for everything else.
///
/// Port of the abstract `CompressingCodec` (`extends FilterCodec`); its
/// subclasses are the codecs of this module, each of which builds one.
pub struct CompressingCodec {
    name: &'static str,
    delegate: Arc<dyn Codec>,
    stored_fields_format: Lucene90CompressingStoredFieldsFormat,
    term_vectors_format: Lucene90CompressingTermVectorsFormat,
}

impl CompressingCodec {
    /// `randomInstance(random, chunkSize, maxDocsPerChunk, withSegmentSuffix,
    /// blockShift)`: one of the six compressing codecs, by
    /// `random.nextInt(6)`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` the formats' constructors throw for
    /// parameters out of range.
    pub fn random_instance_with(
        random: &dyn Random,
        chunk_size: i32,
        max_docs_per_chunk: i32,
        with_segment_suffix: bool,
        block_shift: i32,
    ) -> Result<CompressingCodec> {
        match random.next_int_bound(6) {
            0 => FastCompressingCodec::new(
                chunk_size,
                max_docs_per_chunk,
                with_segment_suffix,
                block_shift,
            ),
            1 => FastDecompressionCompressingCodec::new(
                chunk_size,
                max_docs_per_chunk,
                with_segment_suffix,
                block_shift,
            ),
            2 => HighCompressionCompressingCodec::new(
                chunk_size,
                max_docs_per_chunk,
                with_segment_suffix,
                block_shift,
            ),
            3 => DummyCompressingCodec::new(
                chunk_size,
                max_docs_per_chunk,
                with_segment_suffix,
                block_shift,
            ),
            4 => DeflateWithPresetCompressingCodec::new(
                chunk_size,
                max_docs_per_chunk,
                with_segment_suffix,
                block_shift,
            ),
            5 => LZ4WithPresetCompressingCodec::new(
                chunk_size,
                max_docs_per_chunk,
                with_segment_suffix,
                block_shift,
            ),
            _ => randomizedtesting::jdk::exception::throw_without_message(
                randomizedtesting::jdk::exception::ASSERTION_ERROR,
            ),
        }
    }

    /// `randomInstance(random)`: a random compressing codec that is using an
    /// empty segment suffix.
    ///
    /// # Errors
    ///
    /// As [`random_instance_with`](Self::random_instance_with).
    pub fn random_instance(random: &dyn Random) -> Result<CompressingCodec> {
        let chunk_size = if random.next_boolean() {
            RandomNumbers::random_int_between(random, 10, 100)
        } else {
            RandomNumbers::random_int_between(random, 10, 1 << 15)
        };
        let chunk_docs = if random.next_boolean() {
            RandomNumbers::random_int_between(random, 1, 10)
        } else {
            RandomNumbers::random_int_between(random, 64, 1024)
        };
        let block_size = if random.next_boolean() {
            RandomNumbers::random_int_between(random, DirectMonotonicWriter::MIN_BLOCK_SHIFT, 10)
        } else {
            RandomNumbers::random_int_between(
                random,
                DirectMonotonicWriter::MIN_BLOCK_SHIFT,
                DirectMonotonicWriter::MAX_BLOCK_SHIFT,
            )
        };
        Self::random_instance_with(random, chunk_size, chunk_docs, false, block_size)
    }

    /// `reasonableInstance(random)`: a random compressing codec with more
    /// reasonable parameters for big tests.
    ///
    /// # Errors
    ///
    /// As [`random_instance_with`](Self::random_instance_with).
    pub fn reasonable_instance(random: &dyn Random) -> Result<CompressingCodec> {
        // e.g. defaults use 2^14 for FAST and ~ 2^16 for HIGH
        let chunk_size = TestUtil::next_int(random, 1 << 13, 1 << 17);
        // e.g. defaults use 128 for FAST and 512 for HIGH
        let chunk_docs = TestUtil::next_int(random, 1 << 6, 1 << 10);
        // e.g. defaults use 1024 for both cases
        let block_shift = TestUtil::next_int(random, 8, 12);
        Self::random_instance_with(random, chunk_size, chunk_docs, false, block_shift)
    }

    /// `randomInstance(random, withSegmentSuffix)`: a random compressing
    /// codec that is using a segment suffix (when `with_segment_suffix`).
    ///
    /// # Errors
    ///
    /// As [`random_instance_with`](Self::random_instance_with): the block
    /// shift is drawn in `[1, 1024]`, so Java's stored-fields constructor
    /// refuses most draws.
    pub fn random_instance_with_suffix(
        random: &dyn Random,
        with_segment_suffix: bool,
    ) -> Result<CompressingCodec> {
        let chunk_size = RandomNumbers::random_int_between(random, 1, 1 << 15);
        let max_docs_per_chunk = RandomNumbers::random_int_between(random, 64, 1024);
        let block_shift = RandomNumbers::random_int_between(random, 1, 1024);
        Self::random_instance_with(
            random,
            chunk_size,
            max_docs_per_chunk,
            with_segment_suffix,
            block_shift,
        )
    }

    /// `new CompressingCodec(name, segmentSuffix, compressionMode, chunkSize,
    /// maxDocsPerChunk, blockShift)`: a compressing codec with a given
    /// segment suffix, over `TestUtil.getDefaultCodec()`.
    ///
    /// # Errors
    ///
    /// The `IllegalArgumentException` the formats' constructors throw for
    /// parameters out of range (Java's `int`s; a negative chunk size or
    /// document count is refused as below 1).
    pub fn new(
        name: &'static str,
        segment_suffix: &str,
        compression_mode: Arc<dyn CompressionMode>,
        chunk_size: i32,
        max_docs_per_chunk: i32,
        block_shift: i32,
    ) -> Result<CompressingCodec> {
        let stored_fields_format = Lucene90CompressingStoredFieldsFormat::with_segment_suffix(
            name,
            segment_suffix,
            Arc::clone(&compression_mode),
            chunk_size.max(0) as usize,
            max_docs_per_chunk.max(0) as usize,
            block_shift,
        )?;
        let term_vectors_format = Lucene90CompressingTermVectorsFormat::new(
            name,
            segment_suffix,
            compression_mode,
            chunk_size,
            max_docs_per_chunk,
            block_shift,
        )?;
        Ok(CompressingCodec {
            name,
            delegate: TestUtil::get_default_codec(),
            stored_fields_format,
            term_vectors_format,
        })
    }

    /// `new CompressingCodec(name, compressionMode, chunkSize,
    /// maxDocsPerChunk, blockSize)`: a compressing codec with an empty
    /// segment suffix.
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn without_suffix(
        name: &'static str,
        compression_mode: Arc<dyn CompressionMode>,
        chunk_size: i32,
        max_docs_per_chunk: i32,
        block_size: i32,
    ) -> Result<CompressingCodec> {
        Self::new(
            name,
            "",
            compression_mode,
            chunk_size,
            max_docs_per_chunk,
            block_size,
        )
    }
}

impl fmt::Display for CompressingCodec {
    /// `CompressingCodec.toString()` (`CompressingCodec.java:147-155`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}(storedFieldsFormat={}, termVectorsFormat={})",
            self.name, self.stored_fields_format, self.term_vectors_format
        )
    }
}

impl fmt::Debug for CompressingCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Codec for CompressingCodec {
    fn name(&self) -> &str {
        self.name
    }

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        self.delegate.postings_format()
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.delegate.doc_values_format()
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        &self.stored_fields_format
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        &self.term_vectors_format
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.delegate.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.delegate.segment_info_format()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        self.delegate.norms_format()
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.delegate.live_docs_format()
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.delegate.compound_format()
    }

    fn points_format(&self) -> &dyn PointsFormat {
        self.delegate.points_format()
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        self.delegate.knn_vectors_format()
    }
}
