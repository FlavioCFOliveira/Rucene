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

//! Port of `org.apache.lucene.tests.index.RandomCodec`.

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex, PoisonError};

use randomizedtesting::jdk::lang::{double_to_string, string_hash_code};
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::util::shuffle;
use rucene::codecs::lucene90::points::{
    Lucene90PointsReader, Lucene90PointsWriter, VERSION_CURRENT,
};
use rucene::codecs::points::{PointsFormat, PointsReader, PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{
    Codec, CompoundFormat, DocValuesFormat, FieldInfosFormat, KnnVectorsFormat, LiveDocsFormat,
    Lucene90DocValuesFormat, NormsFormat, PostingsFormat, SegmentInfoFormat, StoredFieldsFormat,
    TermVectorsFormat,
};
use rucene::error::Result;
use rucene::util::java_collections::{JavaHashMap, JavaHashSet};
use rucene_codecs::blocktreeords::BlockTreeOrdsPostingsFormat;
use rucene_codecs::memory::{DirectPostingsFormat, FSTPostingsFormat};

use crate::codecs::asserting::{
    AssertingCodec, AssertingDocValuesFormat, AssertingPointsFormat, AssertingPostingsFormat,
};
use crate::codecs::blockterms::{
    LuceneFixedGap, LuceneVarGapDocFreqInterval, LuceneVarGapFixedInterval,
};
use crate::codecs::bloom::TestBloomFilteredLucenePostings;
use crate::codecs::mockrandom::MockRandomPostingsFormat;
use crate::util::lucene_test_case::LuceneTestCase;
use crate::util::TestUtil;

/// The per-field state `getPostingsFormatForField` and
/// `getDocValuesFormatForField` share with the `AssertingCodec` closures.
struct Mappings {
    /// Shuffled list of postings formats to use for new mappings.
    formats: Vec<Arc<dyn PostingsFormat>>,
    /// Shuffled list of docvalues formats to use for new mappings.
    dv_formats: Vec<Arc<dyn DocValuesFormat>>,
    /// memorized field to postingsformat mappings (`Collections.synchronizedMap(new HashMap<>())`)
    previous_mappings: Mutex<JavaHashMap<Arc<dyn PostingsFormat>>>,
    previous_dv_mappings: Mutex<JavaHashMap<Arc<dyn DocValuesFormat>>>,
    per_field_seed: i32,
}

/// `Math.abs(perFieldSeed ^ name.hashCode()) % size`, then `List.get`: Java's
/// `Math.abs(Integer.MIN_VALUE)` stays negative, and `get` then throws.
fn pick_index(per_field_seed: i32, name: &str, size: usize) -> usize {
    let idx = (per_field_seed ^ string_hash_code(name)).wrapping_abs() % size as i32;
    if idx < 0 {
        panic!("java.lang.IndexOutOfBoundsException: Index {idx} out of bounds for length {size}");
    }
    idx as usize
}

impl Mappings {
    /// `getPostingsFormatForField(name)`.
    fn postings_format_for_field(&self, name: &str) -> Arc<dyn PostingsFormat> {
        let mut previous = self
            .previous_mappings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(codec) = previous.get(name) {
            return Arc::clone(codec);
        }
        let codec =
            Arc::clone(&self.formats[pick_index(self.per_field_seed, name, self.formats.len())]);
        previous.insert(name.to_string(), Arc::clone(&codec));
        // Safety:
        randomizedtesting::jdk::exception::java_assert(previous.len() < 10000, || {
            "test went insane".to_string()
        });
        codec
    }

    /// `getDocValuesFormatForField(name)`.
    fn doc_values_format_for_field(&self, name: &str) -> Arc<dyn DocValuesFormat> {
        let mut previous = self
            .previous_dv_mappings
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(codec) = previous.get(name) {
            return Arc::clone(codec);
        }
        let codec = Arc::clone(
            &self.dv_formats[pick_index(self.per_field_seed, name, self.dv_formats.len())],
        );
        previous.insert(name.to_string(), Arc::clone(&codec));
        // Safety:
        randomizedtesting::jdk::exception::java_assert(previous.len() < 10000, || {
            "test went insane".to_string()
        });
        codec
    }
}

/// `AbstractMap.toString()` of a `HashMap` of formats: `{k=v, k=v}` in the
/// map's iteration order.
fn map_to_string<V: ?Sized>(map: &JavaHashMap<Arc<V>>, to_string: impl Fn(&V) -> String) -> String {
    let entries: Vec<String> = map
        .iter()
        .map(|(k, v)| format!("{k}={}", to_string(&**v)))
        .collect();
    format!("{{{}}}", entries.join(", "))
}

/// Port of `org.apache.lucene.tests.index.RandomCodec` (`extends
/// AssertingCodec`): "Codec that assigns per-field random postings formats.
///
/// The same field/format assignment will happen regardless of order, a hash
/// is computed up front that determines the mapping. This means fields can be
/// put into things like HashSets and added to documents in different orders
/// and the test will still be deterministic and reproducable."
///
/// The superclass is the [`AssertingCodec`] this codec holds, built with its
/// per-field closures over the shared mappings; [`Codec::points_format`] is
/// the override of `pointsFormat()`.
pub struct RandomCodec {
    asserting: AssertingCodec,
    mappings: Arc<Mappings>,
    /// unique set of format names this codec knows about
    pub format_names: JavaHashSet,
    /// unique set of docvalues format names this codec knows about
    pub dv_format_names: JavaHashSet,
    /// `avoidCodecs`.
    pub avoid_codecs: HashSet<String>,
    // a little messy: randomize the default codec's parameters here.
    // with the default values, we have e,g, 512 points in leaf nodes,
    // which is less effective for testing.
    // TODO: improve how we randomize this...
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
    bkd_split_random_seed: i32,
    points_format: AssertingPointsFormat,
}

/// The anonymous `PointsFormat` of `RandomCodec.pointsFormat()`.
struct RandomPointsFormat {
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
    bkd_split_random_seed: i32,
}

impl fmt::Debug for RandomPointsFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RandomPointsFormat")
            .field("max_points_in_leaf_node", &self.max_points_in_leaf_node)
            .field("max_mb_sort_in_heap", &self.max_mb_sort_in_heap)
            .finish_non_exhaustive()
    }
}

impl PointsFormat for RandomPointsFormat {
    /// Java's `PointsFormat` has no name; `Lucene90PointsFormat`'s.
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, write_state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        // Randomize how BKDWriter chooses its splits:
        let mut writer = Lucene90PointsWriter::with_config(
            write_state,
            self.max_points_in_leaf_node,
            self.max_mb_sort_in_heap,
            VERSION_CURRENT,
        )?;
        let bkd_split_random_seed = self.bkd_split_random_seed;
        writer.set_split_override_for_field(Box::new(move |field_info| {
            // `new RandomlySplittingBKDWriter(..., bkdSplitRandomSeed ^
            // fieldInfo.name.hashCode())`.
            let random = JdkRandom::new(i64::from(
                bkd_split_random_seed ^ string_hash_code(&field_info.name),
            ));
            // RandomlySplittingBKDWriter.split: BKD normally defaults by the
            // widest dimension, to try to make as squarish cells as possible,
            // but we just pick a random one ;)
            Box::new(move |_min, _max, _parent_dims, config| {
                random.next_int_bound(config.num_index_dims) as usize
            })
        }));
        Ok(Box::new(writer))
    }

    fn fields_reader(&self, read_state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(read_state)?))
    }
}

impl RandomCodec {
    /// `new RandomCodec(random, avoidCodecs)`.
    ///
    /// # Panics
    ///
    /// When a postings format refuses its parameters, as Java's constructors
    /// throw `IllegalArgumentException`.
    pub fn new_avoiding(random: &dyn Random, avoid_codecs: HashSet<String>) -> Self {
        let per_field_seed = random.next_int();
        // TODO: make it possible to specify min/max iterms per
        // block via CL:
        let min_items_per_block = TestUtil::next_int(random, 2, 100);
        let max_items_per_block = 2 * (2.max(min_items_per_block - 1)) + random.next_int_bound(100);
        let low_freq_cutoff = TestUtil::next_int(random, 2, 100);

        let max_points_in_leaf_node = TestUtil::next_int(random, 16, 2048);
        let max_mb_sort_in_heap = 5.0 + (3.0 * random.next_double());
        let bkd_split_random_seed = random.next_int();

        let rarely = |r: &dyn Random| LuceneTestCase::rarely_with(r);
        // The varargs of `add(...)`, evaluated left to right.
        let postings: Vec<Arc<dyn PostingsFormat>> = vec![
            TestUtil::get_default_postings_format_with(min_items_per_block, max_items_per_block),
            Arc::new(FSTPostingsFormat::new()),
            {
                let min_skip_count = if rarely(random) {
                    1
                } else if rarely(random) {
                    i32::MAX
                } else {
                    max_items_per_block
                };
                let low_freq = if rarely(random) {
                    1
                } else if rarely(random) {
                    i32::MAX
                } else {
                    low_freq_cutoff
                };
                Arc::new(DirectPostingsFormat::with_params(min_skip_count, low_freq))
            },
            // TODO as a PostingsFormat which wraps others, we should allow
            // TestBloomFilteredLucenePostings to be constructed
            // with a choice of concrete PostingsFormats. Maybe useful to have a generic means of
            // marking and dealing
            // with such "wrapper" classes?
            Arc::new(TestBloomFilteredLucenePostings::new()),
            Arc::new(MockRandomPostingsFormat::with_random(random)),
            Arc::new(
                BlockTreeOrdsPostingsFormat::with_block_sizes(
                    min_items_per_block,
                    max_items_per_block,
                )
                .unwrap_or_else(|e| panic!("{e}")),
            ),
            Arc::new(LuceneFixedGap::with_interval(TestUtil::next_int(
                random, 1, 1000,
            ))),
            Arc::new(LuceneVarGapFixedInterval::with_interval(
                TestUtil::next_int(random, 1, 1000),
            )),
            {
                let doc_freq_threshold = TestUtil::next_int(random, 1, 100);
                let term_index_interval = TestUtil::next_int(random, 1, 1000);
                Arc::new(LuceneVarGapDocFreqInterval::with_settings(
                    doc_freq_threshold,
                    term_index_interval,
                ))
            },
            TestUtil::get_default_postings_format(),
            Arc::new(AssertingPostingsFormat::new()),
        ];

        let mut formats = Vec::new();
        let mut format_names = JavaHashSet::new();
        for p in postings {
            if !avoid_codecs.contains(p.name()) {
                format_names.insert(p.name().to_string());
                formats.push(p);
            }
        }

        let doc_values: Vec<Arc<dyn DocValuesFormat>> = vec![
            TestUtil::get_default_doc_values_format(),
            Arc::new(Lucene90DocValuesFormat::new()),
            Arc::new(AssertingDocValuesFormat::new()),
        ];
        let mut dv_formats = Vec::new();
        let mut dv_format_names = JavaHashSet::new();
        for d in doc_values {
            if !avoid_codecs.contains(d.name()) {
                dv_format_names.insert(d.name().to_string());
                dv_formats.push(d);
            }
        }

        shuffle(&mut formats, random);
        shuffle(&mut dv_formats, random);

        // Avoid too many open files:
        formats.truncate(4);
        dv_formats.truncate(4);

        let mappings = Arc::new(Mappings {
            formats,
            dv_formats,
            previous_mappings: Mutex::new(JavaHashMap::new()),
            previous_dv_mappings: Mutex::new(JavaHashMap::new()),
            per_field_seed,
        });
        let postings_for_field = {
            let mappings = Arc::clone(&mappings);
            Arc::new(move |name: &str| mappings.postings_format_for_field(name))
                as crate::codecs::asserting::asserting_codec::PostingsFormatForField
        };
        let doc_values_for_field = {
            let mappings = Arc::clone(&mappings);
            Arc::new(move |name: &str| mappings.doc_values_format_for_field(name))
                as crate::codecs::asserting::asserting_codec::DocValuesFormatForField
        };
        RandomCodec {
            asserting: AssertingCodec::with_formats_for_field(
                Some(postings_for_field),
                Some(doc_values_for_field),
                None,
            ),
            mappings,
            format_names,
            dv_format_names,
            avoid_codecs,
            max_points_in_leaf_node,
            max_mb_sort_in_heap,
            bkd_split_random_seed,
            points_format: AssertingPointsFormat::with_format(Box::new(RandomPointsFormat {
                max_points_in_leaf_node,
                max_mb_sort_in_heap,
                bkd_split_random_seed,
            })),
        }
    }

    /// `new RandomCodec(random)`: `this(random, Collections.emptySet())`.
    pub fn new(random: &dyn Random) -> Self {
        Self::new_avoiding(random, HashSet::new())
    }

    /// `getPostingsFormatForField(name)`.
    pub fn get_postings_format_for_field(&self, name: &str) -> Arc<dyn PostingsFormat> {
        self.mappings.postings_format_for_field(name)
    }

    /// `getDocValuesFormatForField(name)`.
    pub fn get_doc_values_format_for_field(&self, name: &str) -> Arc<dyn DocValuesFormat> {
        self.mappings.doc_values_format_for_field(name)
    }

    /// `getKnnVectorsFormatForField(name)`, inherited from `AssertingCodec`.
    pub fn get_knn_vectors_format_for_field(&self, name: &str) -> Arc<dyn KnnVectorsFormat> {
        self.asserting.get_knn_vectors_format_for_field(name)
    }

    /// The shuffled postings formats new mappings pick from (`formats`).
    pub fn formats(&self) -> &[Arc<dyn PostingsFormat>] {
        &self.mappings.formats
    }

    /// The shuffled doc values formats new mappings pick from (`dvFormats`).
    pub fn dv_formats(&self) -> &[Arc<dyn DocValuesFormat>] {
        &self.mappings.dv_formats
    }

    /// `perFieldSeed`.
    pub fn per_field_seed(&self) -> i32 {
        self.mappings.per_field_seed
    }

    /// `maxPointsInLeafNode`.
    pub fn max_points_in_leaf_node(&self) -> i32 {
        self.max_points_in_leaf_node
    }

    /// `maxMBSortInHeap`.
    pub fn max_mb_sort_in_heap(&self) -> f64 {
        self.max_mb_sort_in_heap
    }

    /// `bkdSplitRandomSeed`.
    pub fn bkd_split_random_seed(&self) -> i32 {
        self.bkd_split_random_seed
    }
}

impl fmt::Display for RandomCodec {
    /// `toString()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let postings = map_to_string(
            &self
                .mappings
                .previous_mappings
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            |p: &(dyn PostingsFormat + 'static)| p.to_java_string(),
        );
        let doc_values = map_to_string(
            &self
                .mappings
                .previous_dv_mappings
                .lock()
                .unwrap_or_else(PoisonError::into_inner),
            |d: &(dyn DocValuesFormat + 'static)| d.to_string(),
        );
        write!(
            f,
            "{}: {postings}, docValues:{doc_values}, maxPointsInLeafNode={}, maxMBSortInHeap={}",
            self.asserting,
            self.max_points_in_leaf_node,
            double_to_string(self.max_mb_sort_in_heap)
        )
    }
}

impl fmt::Debug for RandomCodec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl Codec for RandomCodec {
    fn name(&self) -> &str {
        self.asserting.name()
    }

    fn to_java_string(&self) -> String {
        self.to_string()
    }

    fn postings_format(&self) -> &dyn PostingsFormat {
        self.asserting.postings_format()
    }

    fn doc_values_format(&self) -> &dyn DocValuesFormat {
        self.asserting.doc_values_format()
    }

    fn stored_fields_format(&self) -> &dyn StoredFieldsFormat {
        self.asserting.stored_fields_format()
    }

    fn term_vectors_format(&self) -> &dyn TermVectorsFormat {
        self.asserting.term_vectors_format()
    }

    fn field_infos_format(&self) -> &dyn FieldInfosFormat {
        self.asserting.field_infos_format()
    }

    fn segment_info_format(&self) -> &dyn SegmentInfoFormat {
        self.asserting.segment_info_format()
    }

    fn norms_format(&self) -> &dyn NormsFormat {
        self.asserting.norms_format()
    }

    fn live_docs_format(&self) -> &dyn LiveDocsFormat {
        self.asserting.live_docs_format()
    }

    fn compound_format(&self) -> &dyn CompoundFormat {
        self.asserting.compound_format()
    }

    /// `pointsFormat()`: an `AssertingPointsFormat` over a
    /// `Lucene90PointsWriter` with this codec's `maxPointsInLeafNode` and
    /// `maxMBSortInHeap` whose `writeField` splits cells on random
    /// dimensions, and a `Lucene90PointsReader`. Unlike the superclass's, it
    /// ignores `@SuppressAssertingFormats(POINTS)`.
    fn points_format(&self) -> &dyn PointsFormat {
        &self.points_format
    }

    fn knn_vectors_format(&self) -> &dyn KnnVectorsFormat {
        self.asserting.knn_vectors_format()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `Math.abs(Integer.MIN_VALUE)` stays negative and `List.get` throws.
    #[test]
    fn pick_index_throws_where_math_abs_overflows() {
        let seed = i32::MIN ^ string_hash_code("x");
        // `Integer.MIN_VALUE % 4 == 0`: in bounds, as in Java.
        assert_eq!(pick_index(seed, "x", 4), 0);
        let thrown = std::panic::catch_unwind(|| pick_index(seed, "x", 3)).unwrap_err();
        let message = randomizedtesting::jdk::exception::panic_message(&*thrown);
        assert_eq!(
            message,
            "java.lang.IndexOutOfBoundsException: Index -2 out of bounds for length 3"
        );
        assert_eq!(pick_index(-7 ^ string_hash_code("x"), "x", 4), 3);
    }
}
