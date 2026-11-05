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

//! Port of `org.apache.lucene.tests.codecs.bloom.TestBloomFilteredLucenePostings`.

use std::fmt;
use std::sync::Arc;

use rucene::codecs::{
    FieldsConsumer, FieldsProducer, PostingsFormat, SegmentReadState, SegmentWriteState,
};
use rucene::error::Result;
use rucene::index::FieldInfo;
use rucene_codecs::bloom::{BloomFilterFactory, BloomFilteringPostingsFormat, FuzzySet};

use crate::util::TestUtil;

/// A class used for testing `BloomFilteringPostingsFormat` with a concrete
/// delegate (Lucene41). Creates a Bloom filter on ALL fields and with tiny
/// amounts of memory reserved for the filter. DO NOT USE IN A PRODUCTION
/// APPLICATION This is not a realistic application of Bloom Filters as they
/// ordinarily are larger and operate on only primary key type fields.
///
/// Port of `TestBloomFilteredLucenePostings` (`extends PostingsFormat`,
/// named `TestBloomFilteredLucenePostings`).
#[derive(Debug)]
pub struct TestBloomFilteredLucenePostings {
    delegate: BloomFilteringPostingsFormat,
}

/// Special class used to avoid OOM exceptions where Junit tests create many
/// fields.
///
/// Port of the package-private `TestBloomFilteredLucenePostings.LowMemoryBloomFactory`.
#[derive(Debug, Clone, Copy, Default)]
pub struct LowMemoryBloomFactory;

impl BloomFilterFactory for LowMemoryBloomFactory {
    /// `FuzzySet.createSetBasedOnMaxMemory(1024)` for every field.
    fn get_set_for_field(
        &self,
        _state: &SegmentWriteState<'_>,
        _info: &FieldInfo,
    ) -> Result<Option<FuzzySet>> {
        Ok(Some(FuzzySet::create_set_based_on_max_memory(1024)))
    }

    /// For test purposes always maintain the BloomFilter - even past the
    /// point of usefulness when all bits are set.
    fn is_saturated(&self, _bloom_filter: &FuzzySet, _field_info: &FieldInfo) -> bool {
        false
    }
}

impl TestBloomFilteredLucenePostings {
    /// `new TestBloomFilteredLucenePostings()`: a `BloomFilteringPostingsFormat`
    /// over `TestUtil.getDefaultPostingsFormat()` with a
    /// [`LowMemoryBloomFactory`].
    pub fn new() -> Self {
        TestBloomFilteredLucenePostings {
            delegate: BloomFilteringPostingsFormat::with_factory(
                TestUtil::get_default_postings_format(),
                Arc::new(LowMemoryBloomFactory),
            ),
        }
    }
}

impl Default for TestBloomFilteredLucenePostings {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for TestBloomFilteredLucenePostings {
    /// `toString()`: `"TestBloomFilteredLucenePostings(" + delegate + ")"`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TestBloomFilteredLucenePostings({})", self.delegate)
    }
}

impl PostingsFormat for TestBloomFilteredLucenePostings {
    /// `toString()`: this format's [`fmt::Display`].
    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn name(&self) -> &str {
        "TestBloomFilteredLucenePostings"
    }

    fn fields_consumer<'a>(
        &self,
        state: &SegmentWriteState<'a>,
    ) -> Result<Box<dyn FieldsConsumer + 'a>> {
        self.delegate.fields_consumer(state)
    }

    fn fields_producer<'a>(&self, state: &SegmentReadState<'a>) -> Result<Box<dyn FieldsProducer>> {
        self.delegate.fields_producer(state)
    }
}
