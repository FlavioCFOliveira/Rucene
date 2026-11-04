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

//! The members of `org.apache.lucene.tests.util.TestUtil` that
//! `LuceneTestCase`'s static API and the test codecs
//! (`org.apache.lucene.tests.codecs`) call, ported in full under the sprint's
//! cycle rule; task #342 ports the rest of the class.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::random::Random;
use rucene::codecs::lucene99::hnsw_vectors_format::{
    DEFAULT_BEAM_WIDTH, DEFAULT_MAX_CONN, DEFAULT_NUM_MERGE_WORKER,
};
use rucene::codecs::{
    Codec, DocValuesFormat, KnnVectorsFormat, Lucene104Codec, Lucene104PostingsFormat,
    Lucene90DocValuesFormat, Lucene99HnswVectorsFormat, PostingsFormat,
};

/// Port of `org.apache.lucene.tests.util.TestUtil` (the members
/// `LuceneTestCase` calls).
pub struct TestUtil;

impl TestUtil {
    /// `getDefaultCodec()`: the actual default codec (e.g.
    /// `LuceneMNCodec`) for this version of Lucene, `new Lucene104Codec()`.
    pub fn get_default_codec() -> Arc<dyn Codec> {
        Arc::new(Lucene104Codec::new())
    }

    /// `getDefaultPostingsFormat()`: the actual default postings format
    /// (e.g. `LuceneMNPostingsFormat`), `new Lucene104PostingsFormat()`.
    pub fn get_default_postings_format() -> Arc<dyn PostingsFormat> {
        Arc::new(Lucene104PostingsFormat::new())
    }

    /// `getDefaultPostingsFormat(minItemsPerBlock, maxItemsPerBlock)`:
    /// `new Lucene104PostingsFormat(minItemsPerBlock, maxItemsPerBlock)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for block sizes the block-tree writer
    /// refuses, as Java's constructor throws.
    pub fn get_default_postings_format_with(
        min_items_per_block: i32,
        max_items_per_block: i32,
    ) -> Arc<dyn PostingsFormat> {
        Arc::new(Lucene104PostingsFormat::with_block_sizes(
            min_items_per_block,
            max_items_per_block,
        ))
    }

    /// `getDefaultDocValuesFormat()`: the actual default doc-values format,
    /// `new Lucene90DocValuesFormat()`.
    pub fn get_default_doc_values_format() -> Arc<dyn DocValuesFormat> {
        Arc::new(Lucene90DocValuesFormat::new())
    }

    /// `getDefaultKnnVectorsFormat()`: `new
    /// Lucene99HnswVectorsFormat(DEFAULT_MAX_CONN, DEFAULT_BEAM_WIDTH, 0)`,
    /// whose `tinySegmentsThreshold` of 0 builds a graph for every segment.
    pub fn get_default_knn_vectors_format() -> Arc<dyn KnnVectorsFormat> {
        Arc::new(Lucene99HnswVectorsFormat::with_merge_exec_and_threshold(
            DEFAULT_MAX_CONN,
            DEFAULT_BEAM_WIDTH,
            DEFAULT_NUM_MERGE_WORKER,
            None,
            0,
        ))
    }

    /// `nextInt(r, start, end)`: a number in `[start, end]`,
    /// `RandomNumbers.randomIntBetween(r, start, end)`.
    ///
    /// # Panics
    ///
    /// As `RandomNumbers.randomIntBetween`.
    pub fn next_int<R: Random + ?Sized>(r: &R, start: i32, end: i32) -> i32 {
        RandomNumbers::random_int_between(r, start, end)
    }

    /// `floatUlpEquals(x, y, maxUlps)`: whether `x` and `y` are within
    /// `maxUlps` units in the last place, over their raw bits.
    pub fn float_ulp_equals(x: f32, y: f32, max_ulps: i16) -> bool {
        let x_int = x.to_bits() as i32;
        let y_int = y.to_bits() as i32;
        let max_ulps = i32::from(max_ulps);
        if (x_int ^ y_int) < 0 {
            // Numbers have opposite signs, take care of overflow. Remove the
            // sign bit to obtain the absolute ULP above zero.
            let delta_plus = x_int & i32::MAX;
            let delta_minus = y_int & i32::MAX;
            // If either value is NaN, the exponent bits are set to (255 << 23)
            // and the distance above 0.0 is always above a short ULP error.
            // Avoid possible overflow from adding the deltas by splitting the
            // comparison.
            return delta_plus <= max_ulps && delta_minus <= (max_ulps - delta_plus);
        }
        // Numbers have same sign, there is no risk of overflow.
        x_int.wrapping_sub(y_int).wrapping_abs() <= max_ulps && !x.is_nan() && !y.is_nan()
    }

    /// `doubleUlpEquals(x, y, maxUlps)`.
    pub fn double_ulp_equals(x: f64, y: f64, max_ulps: i32) -> bool {
        let x_int = x.to_bits() as i64;
        let y_int = y.to_bits() as i64;
        let max_ulps = i64::from(max_ulps);
        if (x_int ^ y_int) < 0 {
            let delta_plus = x_int & i64::MAX;
            let delta_minus = y_int & i64::MAX;
            return delta_plus <= max_ulps && delta_minus <= (max_ulps - delta_plus);
        }
        x_int.wrapping_sub(y_int).wrapping_abs() <= max_ulps && !x.is_nan() && !y.is_nan()
    }
}
