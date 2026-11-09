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

//! Port of `org.apache.lucene.tests.index.BasePointsFormatTestCase`:
//! abstract class to do basic tests for a points format. NOTE: This test
//! focuses on the points impl, nothing else. The [stretch] goal is for this
//! test to be so thorough in testing a new PointsFormat that if this test
//! passes, then all Lucene tests should also pass. Ie, if there is some bug
//! in a given PointsFormat that this test fails to catch then this test needs
//! to be improved!
//!
//! The class is declared by [`class()`] and its members are the trait
//! [`BasePointsFormatTestCase`]; see
//! [`base_index_file_format_test_case`](super::base_index_file_format_test_case)
//! for how a suite runs.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `testBigIntNDims`, `randomBigInt`: Rust has no `java.math.BigInteger`.
//!   The values are at most `numBytesPerDim * 8 - 1 <= 127` bits wide, so an
//!   `i128` holds each exactly and compares as `BigInteger.compareTo` does;
//!   `randomBigInt` draws what `new BigInteger(numBits, random())` draws (one
//!   `nextBytes` of `numBytesPerDim` bytes, the top bit masked) and one
//!   `nextBoolean`, and the encoding goes through core's
//!   `NumericUtils::big_int_to_sortable_bytes` on the value's
//!   `BigInteger.toByteArray()` form.
//! * `java.util.BitSet`: a growable bit set over `u64` words with the same
//!   `set`, `get`, `cardinality` and `length`.
//! * The anonymous `IntersectVisitor`s are structs; `visit(docID)` throwing
//!   `IllegalStateException` returns that error.

use std::cmp::Ordering;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, LazyLock};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::{modifier, Class};
use randomizedtesting::junit::Assert;
use rucene::document::{
    BinaryPoint, Document, FieldType, IntPoint, LongPoint, NumericDocValuesField, Store,
    StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::{ConcurrentMergeScheduler, SerialMergeScheduler};
use rucene::index::multi_bits::MultiBits;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::point_values::{
    IntersectVisitor, PointTree, PointValues, Relation, MAX_DIMENSIONS, MAX_INDEX_DIMENSIONS,
    MAX_NUM_BYTES,
};
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::{Bits, BytesRef, NumericUtils};

use super::base_index_file_format_test_case::{
    exception_message, format_test, rethrow, BaseIndexFileFormatTestCase,
};
use super::mismatched_codec_reader::MismatchedCodecReader;
use super::random_index_writer::RandomIndexWriter;
use crate::analysis::MockAnalyzer;
use crate::util::lucene_test_case::{
    LuceneTestCase, Nightly, TestDirectory, TEST_NIGHTLY, VERBOSE,
};
use crate::util::TestUtil;

/// The declaration of `org.apache.lucene.tests.index.BasePointsFormatTestCase`
/// (`public abstract class BasePointsFormatTestCase extends
/// BaseIndexFileFormatTestCase`), with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        let t = |name: &str, body: fn(&dyn BasePointsFormatTestCase) -> Result<()>| {
            format_test(name, move |c| body(points_case(c)))
        };
        Class::builder("org.apache.lucene.tests.index.BasePointsFormatTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&super::base_index_file_format_test_case::class())
            .method(t("testBasic", |c| c.test_basic()))
            .method(t("testMerge", |c| c.test_merge()))
            .method(t("testAllPointDocsDeletedInSegment", |c| {
                c.test_all_point_docs_deleted_in_segment()
            }))
            .method(t("testWithExceptions", |c| c.test_with_exceptions()))
            .method(t("testMultiValued", |c| c.test_multi_valued()))
            .method(t("testAllEqual", |c| c.test_all_equal()))
            .method(t("testOneDimEqual", |c| c.test_one_dim_equal()))
            .method(t("testOneDimTwoValues", |c| c.test_one_dim_two_values()))
            .method(t("testBigIntNDims", |c| c.test_big_int_n_dims()))
            .method(t("testRandomBinaryTiny", |c| c.test_random_binary_tiny()))
            .method(t("testRandomBinaryMedium", |c| {
                c.test_random_binary_medium()
            }))
            .method(t("testRandomBinaryBig", |c| c.test_random_binary_big()).annotate(Nightly))
            .method(t("testAddIndexes", |c| c.test_add_indexes()))
            .method(t("testMergeMissing", |c| c.test_merge_missing()))
            .method(t("testDocCountEdgeCases", |c| {
                c.test_doc_count_edge_cases()
            }))
            .method(t("testRandomDocCount", |c| c.test_random_doc_count()))
            .method(t("testMismatchedFields", |c| c.test_mismatched_fields()))
            .build()
    });
    Arc::clone(&CLASS)
}

/// The instance's subclass as a `BasePointsFormatTestCase`.
fn points_case(c: &dyn BaseIndexFileFormatTestCase) -> &dyn BasePointsFormatTestCase {
    c.as_points().unwrap_or_else(|| {
        Throwable::new(
            "java.lang.IllegalArgumentException",
            "object is not an instance of org.apache.lucene.tests.index.BasePointsFormatTestCase",
        )
        .throw()
    })
}

/// `random()`.
fn random() -> Arc<dyn Random> {
    LuceneTestCase::random()
}

/// `java.util.BitSet`, growable, over `u64` words.
#[derive(Debug, Default, Clone, PartialEq)]
struct JavaBitSet {
    words: Vec<u64>,
}

impl JavaBitSet {
    fn set(&mut self, index: i32) {
        let word = index as usize >> 6;
        if word >= self.words.len() {
            self.words.resize(word + 1, 0);
        }
        self.words[word] |= 1u64 << (index & 63);
    }

    fn get(&self, index: i32) -> bool {
        let word = index as usize >> 6;
        word < self.words.len() && self.words[word] & (1u64 << (index & 63)) != 0
    }

    fn cardinality(&self) -> i32 {
        self.words.iter().map(|w| w.count_ones() as i32).sum()
    }

    /// `length()`: the index of the highest set bit plus one.
    fn length(&self) -> i32 {
        for (i, w) in self.words.iter().enumerate().rev() {
            if *w != 0 {
                return (i * 64) as i32 + 64 - w.leading_zeros() as i32;
            }
        }
        0
    }
}

/// `Arrays.compareUnsigned(a, aFrom, aTo, b, bFrom, bTo)`.
fn compare_unsigned(
    a: &[u8],
    a_from: usize,
    a_to: usize,
    b: &[u8],
    b_from: usize,
    b_to: usize,
) -> Ordering {
    a[a_from..a_to].cmp(&b[b_from..b_to])
}

/// The `IllegalStateException` of `visit(int docID)` in the visitors that
/// expect only `visit(docID, packedValue)`.
fn illegal_state() -> LuceneError {
    LuceneError::illegal_state(String::new())
}

/// The visitor of `testBasic` and `testMerge`: every cell crosses, and each
/// value is the sortable encoding of its document.
struct SeenVisitor {
    seen: JavaBitSet,
}

impl IntersectVisitor for SeenVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(illegal_state())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.seen.set(doc_id);
        Assert::assert_equals(
            i64::from(doc_id),
            i64::from(NumericUtils::sortable_bytes_to_int(packed_value, 0)),
        );
        Ok(())
    }

    fn compare(&self, _min_packed: &[u8], _max_packed: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

/// `random.nextBytes(new byte[len])`.
fn random_bytes(len: usize) -> Vec<u8> {
    let mut b = vec![0u8; len];
    random().next_bytes(&mut b);
    b
}

/// `BigInteger.toByteArray()` of `value`: its minimal two's-complement
/// big-endian form.
fn to_byte_array(value: i128) -> Vec<u8> {
    let bytes = value.to_be_bytes();
    let mut start = 0;
    while start < 15 {
        let b = bytes[start];
        let next_sign = bytes[start + 1] & 0x80;
        if (b == 0x00 && next_sign == 0) || (b == 0xff && next_sign != 0) {
            start += 1;
        } else {
            break;
        }
    }
    bytes[start..].to_vec()
}

/// `new BigInteger(bytes)` of a two's-complement big-endian array.
fn from_byte_array(bytes: &[u8]) -> i128 {
    let mut full = if bytes[0] & 0x80 != 0 {
        [0xffu8; 16]
    } else {
        [0u8; 16]
    };
    full[16 - bytes.len()..].copy_from_slice(bytes);
    i128::from_be_bytes(full)
}

/// `NumericUtils.sortableBytesToBigInt(packed, offset, length)`.
fn sortable_bytes_to_big_int(packed: &[u8], offset: usize, length: usize) -> i128 {
    from_byte_array(&NumericUtils::sortable_bytes_to_big_int(
        packed, offset, length,
    ))
}

/// The visitor of `testBigIntNDims`.
struct BigIntVisitor<'a> {
    hits: &'a mut JavaBitSet,
    doc_base: i32,
    num_dims: usize,
    num_bytes_per_dim: usize,
    query_min: &'a [i128],
    query_max: &'a [i128],
}

impl IntersectVisitor for BigIntVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.hits.set(self.doc_base + doc_id);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        for dim in 0..self.num_dims {
            let x = sortable_bytes_to_big_int(
                packed_value,
                dim * self.num_bytes_per_dim,
                self.num_bytes_per_dim,
            );
            if x < self.query_min[dim] || x > self.query_max[dim] {
                return Ok(());
            }
        }
        self.hits.set(self.doc_base + doc_id);
        Ok(())
    }

    fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Result<Relation> {
        let mut crosses = false;
        for dim in 0..self.num_dims {
            let min = sortable_bytes_to_big_int(
                min_packed,
                dim * self.num_bytes_per_dim,
                self.num_bytes_per_dim,
            );
            let max = sortable_bytes_to_big_int(
                max_packed,
                dim * self.num_bytes_per_dim,
                self.num_bytes_per_dim,
            );
            debug_assert!(max >= min);

            if max < self.query_min[dim] || min > self.query_max[dim] {
                return Ok(Relation::CellOutsideQuery);
            } else if min < self.query_min[dim] || max > self.query_max[dim] {
                crosses = true;
            }
        }

        Ok(if crosses {
            Relation::CellCrossesQuery
        } else {
            Relation::CellInsideQuery
        })
    }
}

/// The visitor of `verify`: hits of a random N-dims rect query, by id.
struct RectVisitor<'a> {
    hits: &'a mut JavaBitSet,
    live_docs: Option<&'a dyn Bits>,
    doc_id_to_id: &'a [i32],
    doc_base: i32,
    num_index_dims: usize,
    num_bytes_per_dim: usize,
    query_min: &'a [Vec<u8>],
    query_max: &'a [Vec<u8>],
}

impl IntersectVisitor for RectVisitor<'_> {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        if self
            .live_docs
            .is_none_or(|l| l.get((self.doc_base + doc_id) as usize))
        {
            self.hits
                .set(self.doc_id_to_id[(self.doc_base + doc_id) as usize]);
        }
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if let Some(l) = self.live_docs {
            if !l.get((self.doc_base + doc_id) as usize) {
                return Ok(());
            }
        }
        let n = self.num_bytes_per_dim;
        for dim in 0..self.num_index_dims {
            if compare_unsigned(
                packed_value,
                dim * n,
                dim * n + n,
                &self.query_min[dim],
                0,
                n,
            ) == Ordering::Less
                || compare_unsigned(
                    packed_value,
                    dim * n,
                    dim * n + n,
                    &self.query_max[dim],
                    0,
                    n,
                ) == Ordering::Greater
            {
                return Ok(());
            }
        }
        self.hits
            .set(self.doc_id_to_id[(self.doc_base + doc_id) as usize]);
        Ok(())
    }

    fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Result<Relation> {
        let mut crosses = false;
        let n = self.num_bytes_per_dim;
        for dim in 0..self.num_index_dims {
            if compare_unsigned(max_packed, dim * n, dim * n + n, &self.query_min[dim], 0, n)
                == Ordering::Less
                || compare_unsigned(min_packed, dim * n, dim * n + n, &self.query_max[dim], 0, n)
                    == Ordering::Greater
            {
                return Ok(Relation::CellOutsideQuery);
            } else if compare_unsigned(min_packed, dim * n, dim * n + n, &self.query_min[dim], 0, n)
                == Ordering::Less
                || compare_unsigned(max_packed, dim * n, dim * n + n, &self.query_max[dim], 0, n)
                    == Ordering::Greater
            {
                crosses = true;
            }
        }
        Ok(if crosses {
            Relation::CellCrossesQuery
        } else {
            Relation::CellInsideQuery
        })
    }
}

/// The visitor of `assertSize`: counts both kinds of visits.
#[derive(Default)]
struct SizeVisitor {
    visit_doc_id_size: i64,
    visit_doc_values_size: i64,
}

impl IntersectVisitor for SizeVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        self.visit_doc_id_size += 1;
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        self.visit_doc_values_size += 1;
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

/// The visitor of `testDocCountEdgeCases` and `testRandomDocCount`: every
/// cell is inside the query.
struct InsideVisitor;

impl IntersectVisitor for InsideVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Ok(())
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Ok(())
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Result<Relation> {
        Ok(Relation::CellInsideQuery)
    }
}

/// The anonymous `PointTree` of `getPointValues`.
struct FixedSizeTree {
    estimated_point_count: i64,
}

impl PointTree for FixedSizeTree {
    fn clone_tree(&self) -> Result<Box<dyn PointTree>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn move_to_child(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        Ok(false)
    }

    fn min_packed_value(&self) -> Result<&[u8]> {
        Ok(&[])
    }

    fn max_packed_value(&self) -> Result<&[u8]> {
        Ok(&[])
    }

    fn size(&self) -> Result<i64> {
        Ok(self.estimated_point_count)
    }

    fn visit_doc_ids(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn visit_doc_values(&mut self, _visitor: &mut dyn IntersectVisitor) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }
}

/// The anonymous `PointValues` of `getPointValues(size, docCount,
/// estimatedPointCount)` (`private`).
struct FixedPointValues {
    size: i64,
    doc_count: i32,
    estimated_point_count: i64,
}

impl PointValues for FixedPointValues {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        Ok(Box::new(FixedSizeTree {
            estimated_point_count: self.estimated_point_count,
        }))
    }

    fn size(&self) -> Result<i64> {
        Ok(self.size)
    }

    fn doc_count(&self) -> Result<i32> {
        Ok(self.doc_count)
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn num_dimensions(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }
}

/// The visitors of `testMismatchedFields`: each value is `expected`, in
/// document order.
struct ExpectedValueVisitor {
    expected: BytesRef,
    expected_doc: i32,
}

impl IntersectVisitor for ExpectedValueVisitor {
    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        Assert::assert_true(
            &self.expected.bytes[self.expected.offset..self.expected.offset + self.expected.length]
                == packed_value,
        );
        Assert::assert_equals(i64::from(self.expected_doc), i64::from(doc_id));
        self.expected_doc += 1;
        Ok(())
    }

    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(String::new()))
    }

    fn compare(&self, _min: &[u8], _max: &[u8]) -> Result<Relation> {
        Ok(Relation::CellCrossesQuery)
    }
}

/// `handlePossiblyFakeException(e)` (`private`): `true` when a message of
/// the cause chain is a random `IOException` of `MockDirectoryWrapper` or a
/// background merge failure; rethrows anything else.
fn handle_possibly_fake_exception(e: LuceneError) -> bool {
    let mut ex: Option<&(dyn std::error::Error + 'static)> = Some(&e);
    while let Some(t) = ex {
        let message = match t.downcast_ref::<LuceneError>() {
            Some(l) => Some(exception_message(l)),
            None => t
                .downcast_ref::<std::io::Error>()
                .and_then(rucene::store::java_nio_file::java_message_of),
        };
        if let Some(message) = message {
            if message.contains("a random IOException")
                || message.contains("background merge hit exception")
            {
                return true;
            }
        }
        ex = t.source();
    }
    rethrow(e)
}

/// `flattenBinaryPoint(value, numDataDims, numBytesPerDim)` (`private`).
fn flatten_binary_point(
    value: &[Vec<u8>],
    num_data_dims: usize,
    num_bytes_per_dim: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; value.len() * num_bytes_per_dim];
    for d in 0..num_data_dims {
        result[d * num_bytes_per_dim..(d + 1) * num_bytes_per_dim]
            .copy_from_slice(&value[d][..num_bytes_per_dim]);
    }
    result
}

/// `getDirectory(numPoints)` (`private`).
fn get_directory(num_points: i32) -> TestDirectory {
    if num_points > 100_000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix("TestBKDTree"))
    } else {
        LuceneTestCase::new_directory()
    }
    // dir = FSDirectory.open(createTempDir());
}

/// `randomBigInt(numBytes)` (`private`): `new BigInteger(numBytes * 8 - 1,
/// random())`, negated half of the time.
fn random_big_int(num_bytes: usize) -> i128 {
    // `BigInteger(numBits, rnd)`: `nextBytes` of `(numBits + 7) / 8` bytes,
    // the excess high bits masked off, read as a positive magnitude.
    let mut magnitude = random_bytes(num_bytes);
    magnitude[0] &= 0x7f;
    let mut x = 0i128;
    for b in magnitude {
        x = (x << 8) | i128::from(b);
    }
    if random().next_boolean() {
        x = -x;
    }
    x
}

/// `ConcurrentMergeSchedulerAccess.setSuppressExceptions(ms)` when the
/// config's scheduler is a `ConcurrentMergeScheduler`.
fn maybe_suppress_exceptions(iwc: &IndexWriterConfig) {
    let ms = iwc.merge_scheduler();
    if let Some(cms) = ms
        .as_any()
        .and_then(|a| a.downcast_ref::<ConcurrentMergeScheduler>())
    {
        cms.set_suppress_exceptions();
    }
}

/// `new IndexWriterConfig(new MockAnalyzer(random()))` when `real`, else
/// `newIndexWriterConfig()`.
fn verify_config(use_real_writer: bool) -> IndexWriterConfig {
    if use_real_writer {
        IndexWriterConfig::with_analyzer(Arc::new(MockAnalyzer::new(&*random())))
    } else {
        LuceneTestCase::new_index_writer_config()
    }
}

/// `IntersectVisitor` over a whole `PointValues` (`values.intersect(v)`).
fn intersect(values: &dyn PointValues, visitor: &mut dyn IntersectVisitor) -> Result<()> {
    values.intersect(visitor)
}

/// Port of the members of `BasePointsFormatTestCase`.
pub trait BasePointsFormatTestCase: BaseIndexFileFormatTestCase {
    /// `addRandomFields(Document)`: the override a subclass's
    /// `add_random_fields` calls.
    fn points_add_random_fields(&self, doc: &mut Document) {
        let num_values = random().next_int_bound(3);
        for _ in 0..num_values {
            doc.add(Box::new(
                IntPoint::new("f", &[random().next_int()]).unwrap_or_else(|e| rethrow(e)),
            ));
        }
    }

    /// `mergeIsStable()`: suppress this test from base class: merges for
    /// BKD trees are not stable because the tree created by merge will have
    /// a different structure than the tree created by adding points
    /// separately.
    fn points_merge_is_stable(&self) -> bool {
        false
    }

    /// The body shared by `testBasic` and `testMerge`: twenty one-dimension
    /// points, a commit after the eleventh when `commit_at_10`.
    fn basic_or_merge(&self, commit_at_10: bool) -> Result<()> {
        let mut dir = get_directory(20);
        {
            let mut iwc = LuceneTestCase::new_index_writer_config();
            iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            let mut point = [0u8; 4];
            for i in 0..20 {
                let mut doc = Document::new();
                NumericUtils::int_to_sortable_bytes(i, &mut point, 0);
                doc.add(Box::new(BinaryPoint::new(
                    "dim",
                    &[BytesRef::new(point.to_vec())],
                )?));
                w.add_document(&doc)?;
                if commit_at_10 && i == 10 {
                    w.commit()?;
                }
            }
            w.force_merge(1)?;
            w.close()?;

            let r = directory_reader::open(dir.as_directory())?;
            let sub = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as _)?;
            let values = sub.get_point_values("dim")?.expect("the field has points");

            // Simple test: make sure intersect can visit every doc:
            let mut visitor = SeenVisitor {
                seen: JavaBitSet::default(),
            };
            intersect(&*values, &mut visitor)?;
            Assert::assert_equals(20i64, i64::from(visitor.seen.cardinality()));
            drop(values);
            drop(sub);
            r.close()?;
        }
        dir.close()
    }

    /// `testBasic()`.
    fn test_basic(&self) -> Result<()> {
        self.basic_or_merge(false)
    }

    /// `testMerge()`.
    fn test_merge(&self) -> Result<()> {
        self.basic_or_merge(true)
    }

    /// `testAllPointDocsDeletedInSegment()`.
    fn test_all_point_docs_deleted_in_segment(&self) -> Result<()> {
        let mut dir = get_directory(20);
        {
            let iwc = LuceneTestCase::new_index_writer_config();
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            let mut point = [0u8; 4];
            for i in 0..10 {
                let mut doc = Document::new();
                NumericUtils::int_to_sortable_bytes(i, &mut point, 0);
                doc.add(Box::new(BinaryPoint::new(
                    "dim",
                    &[BytesRef::new(point.to_vec())],
                )?));
                doc.add(Box::new(NumericDocValuesField::new("id", i64::from(i))));
                doc.add(Box::new(LuceneTestCase::new_string_field(
                    "x",
                    "x",
                    Store::NO,
                )));
                w.add_document(&doc)?;
            }
            w.add_document(&Document::new())?;
            w.delete_documents_by_terms(vec![Term::new("x", BytesRef::new(b"x".to_vec()))])?;
            if random().next_boolean() {
                w.force_merge(1)?;
            }
            w.close()?;
            let r = directory_reader::open(dir.as_directory())?;
            Assert::assert_equals(1i64, i64::from(r.num_docs()));
            let ir: Arc<dyn IndexReader> = Arc::clone(&r) as _;
            let live_docs = MultiBits::get_live_docs(&ir)?;

            for ctx in Arc::clone(&ir).leaves()? {
                let reader = ctx.leaf_reader();
                let values = reader.get_point_values("dim")?;

                let id_values = reader.get_numeric_doc_values("id")?;
                let Some(mut id_values) = id_values else {
                    // this is (surprisingly) OK, because if the random IWC flushes all 10 docs before
                    // the 11th doc is added, and force merge runs, it will drop the 100% deleted
                    // segments, and the "id" field never exists in the final single doc segment
                    continue;
                };
                let mut doc_id_to_id = vec![0i32; reader.max_doc() as usize];
                loop {
                    let doc_id = id_values.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    doc_id_to_id[doc_id as usize] = id_values.long_value()? as i32;
                }

                if let Some(values) = values {
                    struct DeletedVisitor<'a> {
                        seen: JavaBitSet,
                        live_docs: Option<&'a dyn Bits>,
                        doc_id_to_id: &'a [i32],
                    }
                    impl IntersectVisitor for DeletedVisitor<'_> {
                        fn visit(&mut self, _doc_id: i32) -> Result<()> {
                            Err(illegal_state())
                        }
                        fn visit_with_value(
                            &mut self,
                            doc_id: i32,
                            packed_value: &[u8],
                        ) -> Result<()> {
                            if self
                                .live_docs
                                .expect("one document is deleted")
                                .get(doc_id as usize)
                            {
                                self.seen.set(doc_id);
                            }
                            Assert::assert_equals(
                                i64::from(self.doc_id_to_id[doc_id as usize]),
                                i64::from(NumericUtils::sortable_bytes_to_int(packed_value, 0)),
                            );
                            Ok(())
                        }
                        fn compare(&self, _min: &[u8], _max: &[u8]) -> Result<Relation> {
                            Ok(Relation::CellCrossesQuery)
                        }
                    }
                    let mut visitor = DeletedVisitor {
                        seen: JavaBitSet::default(),
                        live_docs: live_docs.as_deref(),
                        doc_id_to_id: &doc_id_to_id,
                    };
                    intersect(&*values, &mut visitor)?;
                    Assert::assert_equals(0i64, i64::from(visitor.seen.cardinality()));
                }
            }
            r.close()?;
        }
        dir.close()
    }

    /// `testWithExceptions()`: make sure we close open files, delete temp
    /// files, etc., on exception.
    fn test_with_exceptions(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(1000);
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
        let num_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
        let num_index_dims = TestUtil::next_int(&*random(), 1, num_dims.min(MAX_INDEX_DIMENSIONS));

        let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);

        for _ in 0..num_docs {
            let mut values = Vec::with_capacity(num_dims as usize);
            for _ in 0..num_dims {
                values.push(random_bytes(num_bytes_per_dim as usize));
            }
            doc_values.push(values);
        }

        // Keep retrying until we 1) we allow a big enough heap, and 2) we hit a random IOExc from
        // MDW:
        let mut done = false;
        while !done {
            let mut dir = LuceneTestCase::new_mock_fs_directory(&LuceneTestCase::create_temp_dir());
            {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    dir.set_random_io_exception_rate(0.05);
                    dir.set_random_io_exception_rate_on_open(0.05);
                    self.verify_in(
                        dir.as_directory(),
                        &doc_values,
                        None,
                        num_dims,
                        num_index_dims,
                        num_bytes_per_dim,
                        true,
                    )
                }));
                match outcome {
                    Ok(Ok(())) => {}
                    Ok(Err(LuceneError::IllegalArgument(message))) => {
                        // This just means we got a too-small maxMB for the maxPointsInLeafNode; just
                        // retry w/ more heap
                        Assert::assert_true(message.contains(
                            "either increase maxMBSortInHeap or decrease maxPointsInLeafNode",
                        ));
                    }
                    Ok(Err(e)) => {
                        // `IllegalStateException` and `IOException`
                        done = handle_possibly_fake_exception(e);
                    }
                    Err(payload) => {
                        let ae = Throwable::from_panic(payload);
                        if ae.is_instance("java.lang.AssertionError")
                            && ae
                                .message()
                                .is_some_and(|m| m.contains("does not exist; files="))
                        {
                            // OK: likely we threw the random IOExc when IW was asserting the commit
                            // files exist
                            done = true;
                        } else {
                            ae.throw()
                        }
                    }
                }
            }
            dir.close()?;
        }
        Ok(())
    }

    /// `testMultiValued()`.
    fn test_multi_valued(&self) -> Result<()> {
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
        let num_dims = TestUtil::next_int(&*random(), 1, MAX_DIMENSIONS);
        let num_index_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS.min(num_dims));

        let num_docs = if *TEST_NIGHTLY {
            LuceneTestCase::at_least(1000)
        } else {
            LuceneTestCase::at_least(100)
        };
        let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::new();
        let mut doc_ids: Vec<i32> = Vec::new();

        for doc_id in 0..num_docs {
            let num_values_in_doc = TestUtil::next_int(&*random(), 1, 5);
            for _ in 0..num_values_in_doc {
                doc_ids.push(doc_id);
                let mut values = Vec::with_capacity(num_dims as usize);
                for _ in 0..num_dims {
                    values.push(random_bytes(num_bytes_per_dim as usize));
                }
                doc_values.push(values);
            }
        }

        self.verify_index_dims(
            &doc_values,
            Some(&doc_ids),
            num_dims,
            num_index_dims,
            num_bytes_per_dim,
        )
    }

    /// `testAllEqual()`.
    fn test_all_equal(&self) -> Result<()> {
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
        let num_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS);

        let num_docs = LuceneTestCase::at_least(1000);
        let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);

        for doc_id in 0..num_docs {
            if doc_id == 0 {
                let mut values = Vec::with_capacity(num_dims as usize);
                for _ in 0..num_dims {
                    values.push(random_bytes(num_bytes_per_dim as usize));
                }
                doc_values.push(values);
            } else {
                doc_values.push(doc_values[0].clone());
            }
        }

        self.verify(&doc_values, None, num_dims, num_bytes_per_dim)
    }

    /// `testOneDimEqual()`.
    fn test_one_dim_equal(&self) -> Result<()> {
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
        let num_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS);

        let num_docs = LuceneTestCase::at_least(1000);
        let the_equal_dim = random().next_int_bound(num_dims) as usize;
        let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);

        for doc_id in 0..num_docs {
            let mut values = Vec::with_capacity(num_dims as usize);
            for _ in 0..num_dims {
                values.push(random_bytes(num_bytes_per_dim as usize));
            }
            if doc_id > 0 {
                values[the_equal_dim] = doc_values[0][the_equal_dim].clone();
            }
            doc_values.push(values);
        }

        self.verify(&doc_values, None, num_dims, num_bytes_per_dim)
    }

    /// `testOneDimTwoValues()`: this should trigger run-length compression
    /// with lengths that are greater than 255.
    fn test_one_dim_two_values(&self) -> Result<()> {
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
        let num_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS);

        let num_docs = LuceneTestCase::at_least(1000);
        let the_dim = random().next_int_bound(num_dims);
        let value1 = random_bytes(num_bytes_per_dim as usize);
        let value2 = random_bytes(num_bytes_per_dim as usize);
        let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);

        for _ in 0..num_docs {
            let mut values = Vec::with_capacity(num_dims as usize);
            for dim in 0..num_dims {
                if dim == the_dim {
                    values.push(if random().next_boolean() {
                        value1.clone()
                    } else {
                        value2.clone()
                    });
                } else {
                    values.push(random_bytes(num_bytes_per_dim as usize));
                }
            }
            doc_values.push(values);
        }

        self.verify(&doc_values, None, num_dims, num_bytes_per_dim)
    }

    /// `testBigIntNDims()`: tests on N-dimensional points where each
    /// dimension is a `BigInteger`.
    fn test_big_int_n_dims(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(200);
        let mut dir = get_directory(num_docs);
        {
            let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES) as usize;
            let num_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS) as usize;
            let mut iwc = LuceneTestCase::new_index_writer_config_analyzer(Arc::new(
                MockAnalyzer::new(&*random()),
            ));
            // We rely on docIDs not changing:
            iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
            let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
            let mut docs: Vec<Vec<i128>> = Vec::with_capacity(num_docs as usize);

            for doc_id in 0..num_docs {
                let mut values = Vec::with_capacity(num_dims);
                if *VERBOSE {
                    println!("  docID={doc_id}");
                }
                let mut bytes = Vec::with_capacity(num_dims);
                for dim in 0..num_dims {
                    values.push(random_big_int(num_bytes_per_dim));
                    let mut b = vec![0u8; num_bytes_per_dim];
                    NumericUtils::big_int_to_sortable_bytes(
                        &to_byte_array(values[dim]),
                        num_bytes_per_dim,
                        &mut b,
                        0,
                    );
                    bytes.push(BytesRef::new(b));
                    if *VERBOSE {
                        println!("    {dim} -> {}", values[dim]);
                    }
                }
                docs.push(values);
                let mut doc = Document::new();
                doc.add(Box::new(BinaryPoint::new("field", &bytes)?));
                w.add_document(&doc)?;
            }

            let r = w.get_reader()?;
            w.close()?;

            let iters = LuceneTestCase::at_least(100);
            for iter in 0..iters {
                if *VERBOSE {
                    println!("\nTEST: iter={iter}");
                }

                // Random N dims rect query:
                let mut query_min = vec![0i128; num_dims];
                let mut query_max = vec![0i128; num_dims];
                for dim in 0..num_dims {
                    query_min[dim] = random_big_int(num_bytes_per_dim);
                    query_max[dim] = random_big_int(num_bytes_per_dim);
                    if query_min[dim] > query_max[dim] {
                        query_min.swap(dim, dim);
                        let x = query_min[dim];
                        query_min[dim] = query_max[dim];
                        query_max[dim] = x;
                    }
                    if *VERBOSE {
                        println!(
                            "  {dim}\n    min={}\n    max={}",
                            query_min[dim], query_max[dim]
                        );
                    }
                }

                let mut hits = JavaBitSet::default();
                for ctx in Arc::clone(&r).leaves()? {
                    let Some(dim_values) = ctx.leaf_reader().get_point_values("field")? else {
                        continue;
                    };

                    let doc_base = ctx.doc_base();

                    let mut visitor = BigIntVisitor {
                        hits: &mut hits,
                        doc_base,
                        num_dims,
                        num_bytes_per_dim,
                        query_min: &query_min,
                        query_max: &query_max,
                    };
                    intersect(&*dim_values, &mut visitor)?;
                }

                for doc_id in 0..num_docs {
                    let doc_values = &docs[doc_id as usize];
                    let mut expected = true;
                    for dim in 0..num_dims {
                        let x = doc_values[dim];
                        if x < query_min[dim] || x > query_max[dim] {
                            expected = false;
                            break;
                        }
                    }
                    let actual = hits.get(doc_id);
                    Assert::assert_equals_msg(Some(&format!("docID={doc_id}")), expected, actual);
                }
            }
            r.close()?;
        }
        dir.close()
    }

    /// `testRandomBinaryTiny()`.
    fn test_random_binary_tiny(&self) -> Result<()> {
        self.do_test_random_binary(10)
    }

    /// `testRandomBinaryMedium()`.
    fn test_random_binary_medium(&self) -> Result<()> {
        self.do_test_random_binary(200)
    }

    /// `testRandomBinaryBig()` (`@Nightly`).
    fn test_random_binary_big(&self) -> Result<()> {
        LuceneTestCase::assume_false(
            "too slow with SimpleText",
            rucene::codecs::get_default().is_some_and(|c| c.name() == "SimpleText"),
        );
        self.do_test_random_binary(200_000)
    }

    /// `doTestRandomBinary(count)` (`private`).
    fn do_test_random_binary(&self, count: i32) -> Result<()> {
        let num_docs = TestUtil::next_int(&*random(), count, count * 2);
        let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
        let num_data_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS);
        let num_index_dims = TestUtil::next_int(&*random(), 1, num_data_dims);

        let mut doc_values: Vec<Vec<Vec<u8>>> = Vec::with_capacity(num_docs as usize);

        for _ in 0..num_docs {
            let mut values = Vec::with_capacity(num_data_dims as usize);
            for _ in 0..num_data_dims {
                // TODO: sometimes test on a "small" volume too, so we test the high density cases,
                // higher chance of boundary, etc. cases:
                values.push(random_bytes(num_bytes_per_dim as usize));
            }
            doc_values.push(values);
        }

        self.verify_index_dims(
            &doc_values,
            None,
            num_data_dims,
            num_index_dims,
            num_bytes_per_dim,
        )
    }

    /// `verify(docValues, docIDs, numDims, numBytesPerDim)` (`private`).
    fn verify(
        &self,
        doc_values: &[Vec<Vec<u8>>],
        doc_ids: Option<&[i32]>,
        num_dims: i32,
        num_bytes_per_dim: i32,
    ) -> Result<()> {
        self.verify_index_dims(doc_values, doc_ids, num_dims, num_dims, num_bytes_per_dim)
    }

    /// `verify(docValues, docIDs, numDataDims, numIndexDims,
    /// numBytesPerDim)` (`private`): docIDs can be null, for the single
    /// valued case, else it maps value to docID, but all values for one doc
    /// must be adjacent.
    fn verify_index_dims(
        &self,
        doc_values: &[Vec<Vec<u8>>],
        doc_ids: Option<&[i32]>,
        num_data_dims: i32,
        num_index_dims: i32,
        num_bytes_per_dim: i32,
    ) -> Result<()> {
        let mut dir = get_directory(doc_values.len() as i32);
        {
            loop {
                match self.verify_in(
                    dir.as_directory(),
                    doc_values,
                    doc_ids,
                    num_data_dims,
                    num_index_dims,
                    num_bytes_per_dim,
                    false,
                ) {
                    Ok(()) => break,
                    Err(LuceneError::IllegalArgument(message)) => {
                        println!("java.lang.IllegalArgumentException: {message}");
                        // This just means we got a too-small maxMB for the maxPointsInLeafNode; just
                        // retry
                        Assert::assert_true(message.contains(
                            "either increase maxMBSortInHeap or decrease maxPointsInLeafNode",
                        ));
                    }
                    Err(e) => {
                        let _ = dir.close();
                        return Err(e);
                    }
                }
            }
        }
        dir.close()
    }

    /// `verify(dir, docValues, ids, numDims, numIndexDims, numBytesPerDim,
    /// expectExceptions)` (`private`): test selective indexing.
    #[allow(clippy::too_many_arguments)]
    fn verify_in(
        &self,
        dir: Arc<dyn Directory>,
        doc_values: &[Vec<Vec<u8>>],
        ids: Option<&[i32]>,
        num_dims: i32,
        num_index_dims: i32,
        num_bytes_per_dim: i32,
        expect_exceptions: bool,
    ) -> Result<()> {
        let num_values = doc_values.len();
        if *VERBOSE {
            println!(
                "TEST: numValues={num_values} numDims={num_dims} numIndexDims={num_index_dims} numBytesPerDim={num_bytes_per_dim}"
            );
        }
        let nd = num_dims as usize;
        let nid = num_index_dims as usize;
        let nb = num_bytes_per_dim as usize;

        // RandomIndexWriter is too slow:
        let use_real_writer = doc_values.len() > 10000;

        let iwc = verify_config(use_real_writer);

        if expect_exceptions {
            maybe_suppress_exceptions(&iwc);
        }
        // `dir` is the caller's directory; `side_dir` the separate one Java
        // assigns to `dir` while `saveDir` holds the caller's.
        let mut side_dir: Option<TestDirectory> = None;
        let mut w = RandomIndexWriter::new_with_config(&*random(), Arc::clone(&dir), iwc)?;
        let mut r: Option<Arc<dyn DirectoryReader>> = None;

        // Compute actual min/max values:
        let mut expected_min_values: Vec<Vec<u8>> = vec![Vec::new(); nd];
        let mut expected_max_values: Vec<Vec<u8>> = vec![Vec::new(); nd];
        for (ord, values) in doc_values.iter().enumerate() {
            for dim in 0..nd {
                if ord == 0 {
                    expected_min_values[dim] = values[dim][..nb].to_vec();
                    expected_max_values[dim] = values[dim][..nb].to_vec();
                } else {
                    // TODO: it's cheating that we use StringHelper.compare for "truth": what if
                    // it's buggy?
                    if compare_unsigned(&values[dim], 0, nb, &expected_min_values[dim], 0, nb)
                        == Ordering::Less
                    {
                        expected_min_values[dim] = values[dim][..nb].to_vec();
                    }
                    if compare_unsigned(&values[dim], 0, nb, &expected_max_values[dim], 0, nb)
                        == Ordering::Greater
                    {
                        expected_max_values[dim] = values[dim][..nb].to_vec();
                    }
                }
            }
        }

        // 20% of the time we add into a separate directory, then at some point use
        // addIndexes to bring the indexed point values to the main directory:
        let mut save_w: Option<RandomIndexWriter>;
        let add_indexes_at;
        if random().next_int_bound(5) == 1 {
            save_w = Some(w);
            let d = get_directory(num_values as i32);
            let iwc = verify_config(use_real_writer);
            if expect_exceptions {
                maybe_suppress_exceptions(&iwc);
            }
            w = RandomIndexWriter::new_with_config(&*random(), d.as_directory(), iwc)?;
            side_dir = Some(d);
            add_indexes_at = TestUtil::next_int(&*random(), 1, num_values as i32 - 1);
        } else {
            save_w = None;
            add_indexes_at = 0;
        }

        let body = (|| -> Result<()> {
            let mut field_type = FieldType::new();
            field_type.set_dimensions_with_index_count(
                num_dims,
                num_index_dims,
                num_bytes_per_dim,
            )?;
            field_type.freeze();

            let mut doc: Option<Document> = None;
            let mut last_id = -1;
            for ord in 0..num_values {
                let id = match ids {
                    None => ord as i32,
                    Some(ids) => ids[ord],
                };
                if id != last_id {
                    if let Some(d) = doc.take() {
                        if use_real_writer {
                            w.w().add_document(&d)?;
                        } else {
                            w.add_document(&d)?;
                        }
                    }
                    let mut d = Document::new();
                    d.add(Box::new(NumericDocValuesField::new("id", i64::from(id))));
                    doc = Some(d);
                }
                // pack the binary point
                let val = flatten_binary_point(&doc_values[ord], nd, nb);

                doc.as_mut().expect("a document is open").add(Box::new(
                    BinaryPoint::new_with_packed("field", BytesRef::new(val), field_type.clone())?,
                ));
                last_id = id;

                if random().next_int_bound(30) == 17 {
                    // randomly index some documents without this field
                    if use_real_writer {
                        w.w().add_document(&Document::new())?;
                    } else {
                        w.add_document(&Document::new())?;
                    }
                    if *VERBOSE {
                        println!("add empty doc");
                    }
                }

                if random().next_int_bound(30) == 17 {
                    // randomly index some documents with this field, but we will delete them:
                    let mut xdoc = Document::new();
                    let val = flatten_binary_point(&doc_values[ord], nd, nb);
                    xdoc.add(Box::new(BinaryPoint::new_with_packed(
                        "field",
                        BytesRef::new(val),
                        field_type.clone(),
                    )?));
                    xdoc.add(Box::new(StringField::new(
                        "nukeme",
                        "yes".to_string(),
                        Store::NO,
                    )?));
                    if use_real_writer {
                        w.w().add_document(&xdoc)?;
                    } else {
                        w.add_document(&xdoc)?;
                    }
                    if *VERBOSE {
                        println!("add doc doc-to-delete");
                    }

                    if random().next_int_bound(5) == 1 {
                        let nukeme = Term::new("nukeme", BytesRef::new(b"yes".to_vec()));
                        if use_real_writer {
                            w.w().delete_documents_by_terms(vec![nukeme])?;
                        } else {
                            w.delete_documents(nukeme)?;
                        }
                    }
                }

                if *VERBOSE {
                    println!("  ord={ord} id={id}");
                    for dim in 0..nd {
                        println!(
                            "    dim={dim} value={}",
                            BytesRef::new(doc_values[ord][dim].clone())
                        );
                    }
                }

                if save_w.is_some() && ord as i32 >= add_indexes_at {
                    let target = save_w.take().expect("checked above");
                    let mut d = side_dir.take().expect("the separate directory");
                    // `w = saveW` after `switchIndex(w, dir, saveW)`.
                    let old = std::mem::replace(&mut w, target);
                    switch_index(old, &mut d, &w)?;
                }
            }
            w.add_document(doc.as_ref().expect("at least one value"))?;
            w.delete_documents(Term::new("nukeme", BytesRef::new(b"yes".to_vec())))?;

            if random().next_boolean() {
                if *VERBOSE {
                    println!("\nTEST: now force merge");
                }
                w.force_merge(1)?;
            }

            let reader = w.get_reader()?;
            r = Some(Arc::clone(&reader));
            w.close()?;

            if *VERBOSE {
                println!("TEST: reader={reader}");
            }

            let ir: Arc<dyn IndexReader> = Arc::clone(&reader) as _;
            let mut id_values =
                MultiDocValues::get_numeric_values(&ir, "id")?.expect("every document has an id");
            let mut doc_id_to_id = vec![0i32; reader.max_doc() as usize];
            loop {
                let doc_id = id_values.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                doc_id_to_id[doc_id as usize] = id_values.long_value()? as i32;
            }

            let live_docs = MultiBits::get_live_docs(&ir)?;

            // Verify min/max values are correct:
            let mut min_values = vec![0xffu8; nid * nb];
            let mut max_values = vec![0u8; nid * nb];

            for ctx in Arc::clone(&ir).leaves()? {
                let Some(dim_values) = ctx.leaf_reader().get_point_values("field")? else {
                    continue;
                };
                assert_size(&mut *dim_values.point_tree()?)?;
                let leaf_min_values = dim_values.min_packed_value()?.expect("the leaf has points");
                let leaf_max_values = dim_values.max_packed_value()?.expect("the leaf has points");
                for dim in 0..nid {
                    let (from, to) = (dim * nb, dim * nb + nb);
                    if compare_unsigned(&leaf_min_values, from, to, &min_values, from, to)
                        == Ordering::Less
                    {
                        min_values[from..to].copy_from_slice(&leaf_min_values[from..to]);
                    }
                    if compare_unsigned(&leaf_max_values, from, to, &max_values, from, to)
                        == Ordering::Greater
                    {
                        max_values[from..to].copy_from_slice(&leaf_max_values[from..to]);
                    }
                }
            }

            for dim in 0..nid {
                let scratch = &min_values[dim * nb..dim * nb + nb];
                Assert::assert_true(expected_min_values[dim][..] == *scratch);
                let scratch = &max_values[dim * nb..dim * nb + nb];
                Assert::assert_true(expected_max_values[dim][..] == *scratch);
            }

            let iters = LuceneTestCase::at_least(100);
            for iter in 0..iters {
                if *VERBOSE {
                    println!("\nTEST: iter={iter}");
                }

                // Random N dims rect query:
                let mut query_min: Vec<Vec<u8>> = Vec::with_capacity(nid);
                let mut query_max: Vec<Vec<u8>> = Vec::with_capacity(nid);
                for _ in 0..nid {
                    let mut min = random_bytes(nb);
                    let mut max = random_bytes(nb);
                    if compare_unsigned(&min, 0, nb, &max, 0, nb) == Ordering::Greater {
                        std::mem::swap(&mut min, &mut max);
                    }
                    query_min.push(min);
                    query_max.push(max);
                }

                if *VERBOSE {
                    for dim in 0..nid {
                        println!(
                            "  dim={dim}\n    queryMin={}\n    queryMax={}",
                            BytesRef::new(query_min[dim].clone()),
                            BytesRef::new(query_max[dim].clone())
                        );
                    }
                }

                let mut hits = JavaBitSet::default();

                for ctx in Arc::clone(&ir).leaves()? {
                    let Some(dim_values) = ctx.leaf_reader().get_point_values("field")? else {
                        continue;
                    };

                    let doc_base = ctx.doc_base();

                    let mut visitor = RectVisitor {
                        hits: &mut hits,
                        live_docs: live_docs.as_deref(),
                        doc_id_to_id: &doc_id_to_id,
                        doc_base,
                        num_index_dims: nid,
                        num_bytes_per_dim: nb,
                        query_min: &query_min,
                        query_max: &query_max,
                    };
                    intersect(&*dim_values, &mut visitor)?;
                }

                let mut expected = JavaBitSet::default();
                for ord in 0..num_values {
                    let mut matches = true;
                    for dim in 0..nid {
                        let x = &doc_values[ord][dim];
                        if compare_unsigned(x, 0, nb, &query_min[dim], 0, nb) == Ordering::Less
                            || compare_unsigned(x, 0, nb, &query_max[dim], 0, nb)
                                == Ordering::Greater
                        {
                            matches = false;
                            break;
                        }
                    }

                    if matches {
                        let id = match ids {
                            None => ord as i32,
                            Some(ids) => ids[ord],
                        };
                        expected.set(id);
                    }
                }

                let limit = expected.length().max(hits.length());
                let mut fail_count = 0;
                let mut success_count = 0;
                for id in 0..limit {
                    if expected.get(id) != hits.get(id) {
                        println!("FAIL: id={id}");
                        fail_count += 1;
                    } else {
                        success_count += 1;
                    }
                }

                if fail_count != 0 {
                    for (doc_id, id) in doc_id_to_id.iter().enumerate() {
                        println!("  docID={doc_id} id={id}");
                    }

                    Assert::fail(&format!(
                        "{fail_count} docs failed; {success_count} docs succeeded"
                    ));
                }
            }
            Ok(())
        })();
        // `IOUtils.closeWhileHandlingException(r, w, saveW, saveDir == null ? null : dir)`
        if let Some(r) = r {
            let _ = r.close();
        }
        let _ = w.close();
        if let Some(save_w) = save_w {
            let _ = save_w.close();
        }
        if let Some(mut d) = side_dir {
            let _ = d.close();
        }
        drop(dir);
        body
    }

    /// `testAddIndexes()`.
    fn test_add_indexes(&self) -> Result<()> {
        let mut dir1 = LuceneTestCase::new_directory();
        // Created where Java creates them; declared here so they outlive the
        // block that releases every handle on them before they close.
        let mut dir2;
        let mut dir;
        {
            let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir1.as_directory())?;
            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("int1", &[17])?));
            w.add_document(&doc)?;
            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("int2", &[42])?));
            w.add_document(&doc)?;
            w.close()?;

            // Different field number assigments:
            dir2 = LuceneTestCase::new_directory();
            let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir2.as_directory())?;
            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("int2", &[42])?));
            w.add_document(&doc)?;
            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("int1", &[17])?));
            w.add_document(&doc)?;
            w.close()?;

            dir = LuceneTestCase::new_directory();
            {
                let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
                w.add_indexes_dirs(&[dir1.as_directory(), dir2.as_directory()])?;
                w.force_merge(1)?;

                let r = w.get_reader()?;
                let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
                Assert::assert_equals(
                    2i64,
                    i64::from(s.count(IntPoint::new_exact_query("int1", 17)?)?),
                );
                Assert::assert_equals(
                    2i64,
                    i64::from(s.count(IntPoint::new_exact_query("int2", 42)?)?),
                );
                drop(s);
                r.close()?;
                w.close()?;
            }
            dir.close()?;
        }
        dir1.close()?;
        dir2.close()
    }

    /// `testMergeMissing()` (LUCENE-7491).
    fn test_merge_missing(&self) -> Result<()> {
        let mut dir = LuceneTestCase::new_directory();
        {
            let iwc = LuceneTestCase::new_index_writer_config();
            let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
            // `iwc.setMaxBufferedDocs(2)` after the writer took `iwc`: the
            // writer's live config is that object.
            w.w().get_config().set_max_buffered_docs(2)?;
            for i in 0..2 {
                let mut doc = Document::new();
                doc.add(Box::new(IntPoint::new("int", &[i])?));
                w.add_document(&doc)?;
            }
            // index has 1 segment now (with 2 docs) and that segment does have points

            let mut doc = Document::new();
            doc.add(Box::new(IntPoint::new("id", &[0])?));
            w.add_document(&doc)?;
            // now we write another segment where the id field does have points:
            w.force_merge(1)?;
            w.close()?;
        }
        dir.close()
    }

    /// `testDocCountEdgeCases()`.
    fn test_doc_count_edge_cases(&self) -> Result<()> {
        let mut visitor = InsideVisitor;
        let values = get_point_values(i64::MAX, 1, i64::MAX);
        let docs = values.estimate_doc_count(&mut visitor)?;
        Assert::assert_equals(1i64, docs);
        let values = get_point_values(i64::MAX, 1, 1);
        let docs = values.estimate_doc_count(&mut visitor)?;
        Assert::assert_equals(1i64, docs);
        let values = get_point_values(i64::MAX, i32::MAX, i64::MAX);
        let docs = values.estimate_doc_count(&mut visitor)?;
        Assert::assert_equals(i64::from(i32::MAX), docs);
        let values = get_point_values(i64::MAX, i32::MAX, i64::MAX / 2);
        let docs = values.estimate_doc_count(&mut visitor)?;
        Assert::assert_equals(i64::from(i32::MAX), docs);
        let values = get_point_values(i64::MAX, i32::MAX, 1);
        let docs = values.estimate_doc_count(&mut visitor)?;
        Assert::assert_equals(1i64, docs);
        Ok(())
    }

    /// `testRandomDocCount()`.
    fn test_random_doc_count(&self) -> Result<()> {
        for _ in 0..100 {
            let size = TestUtil::next_long(&*random(), 1, i64::MAX);
            let max_doc = if size > i64::from(i32::MAX) {
                i32::MAX
            } else {
                size as i32
            };
            let doc_count = TestUtil::next_int(&*random(), 1, max_doc);
            let estimated_point_count = TestUtil::next_long(&*random(), 0, size);
            let values = get_point_values(size, doc_count, estimated_point_count);
            let docs = values.estimate_doc_count(&mut InsideVisitor)?;
            Assert::assert_true(docs <= estimated_point_count);
            Assert::assert_true(docs <= i64::from(max_doc));
            Assert::assert_true(docs >= estimated_point_count / (size / i64::from(doc_count)));
        }
        Ok(())
    }

    /// `testMismatchedFields()`.
    fn test_mismatched_fields(&self) -> Result<()> {
        let mut dir1 = LuceneTestCase::new_directory();
        // Created where Java creates it; declared here so it outlives the
        // block that releases every handle on it before it closes.
        let mut dir2;
        {
            let w1 = IndexWriter::new(
                dir1.as_directory(),
                LuceneTestCase::new_index_writer_config(),
            )?;
            let mut doc = Document::new();
            doc.add(Box::new(LongPoint::new("f", &[1])?));
            doc.add(Box::new(LongPoint::new("g", &[42, 43])?));
            w1.add_document(&doc)?;

            dir2 = LuceneTestCase::new_directory();
            let mut iwc2 = LuceneTestCase::new_index_writer_config();
            iwc2.set_merge_scheduler(Arc::new(SerialMergeScheduler::default()))?;
            let w2 = IndexWriter::new(dir2.as_directory(), iwc2)?;
            w2.add_document(&doc)?;
            w2.commit()?;

            let reader = directory_reader::open_with_writer(&*w1)?;
            w1.close()?;
            let codec_reader: Arc<dyn CodecReader> =
                LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?
                    .as_codec_reader()
                    .expect("a segment reader is a CodecReader");
            w2.add_indexes_codec_readers(&[Arc::new(MismatchedCodecReader::new(
                codec_reader,
                &*random(),
            )?)])?;
            reader.close()?;
            w2.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*w2)?;
            w2.close()?;

            let leaf_reader = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as _)?;
            Assert::assert_equals(2i64, i64::from(leaf_reader.max_doc()));

            let f_points = leaf_reader.get_point_values("f")?.expect("f has points");
            Assert::assert_equals(2i64, f_points.size()?);
            intersect(
                &*f_points,
                &mut ExpectedValueVisitor {
                    expected: LongPoint::pack(&[1])?,
                    expected_doc: 0,
                },
            )?;

            let g_points = leaf_reader.get_point_values("g")?.expect("g has points");
            Assert::assert_equals(2i64, f_points.size()?);
            intersect(
                &*g_points,
                &mut ExpectedValueVisitor {
                    expected: LongPoint::pack(&[42, 43])?,
                    expected_doc: 0,
                },
            )?;

            drop(f_points);
            drop(g_points);
            drop(leaf_reader);
            reader.close()?;
            w2.close()?;
        }
        dir1.close()?;
        dir2.close()
    }
}

/// `assertSize(tree)` (`private`).
fn assert_size(tree: &mut dyn PointTree) -> Result<()> {
    let mut clone = tree.clone_tree()?;
    Assert::assert_equals(clone.size()?, tree.size()?);
    // rarely continue with the clone tree
    if LuceneTestCase::rarely() {
        assert_size_of(&mut *clone)
    } else {
        assert_size_of(tree)
    }
}

/// The body of `assertSize` past the choice of tree.
fn assert_size_of(tree: &mut dyn PointTree) -> Result<()> {
    let mut visitor = SizeVisitor::default();
    if random().next_boolean() {
        tree.visit_doc_ids(&mut visitor)?;
        tree.visit_doc_values(&mut visitor)?;
    } else {
        tree.visit_doc_values(&mut visitor)?;
        tree.visit_doc_ids(&mut visitor)?;
    }
    Assert::assert_equals(visitor.visit_doc_id_size, visitor.visit_doc_values_size);
    Assert::assert_equals(visitor.visit_doc_id_size, tree.size()?);
    if tree.move_to_child()? {
        loop {
            random_point_tree_navigation(tree)?;
            assert_size(tree)?;
            if !tree.move_to_sibling()? {
                break;
            }
        }
        tree.move_to_parent()?;
    }
    Ok(())
}

/// `randomPointTreeNavigation(tree)` (`private`).
fn random_point_tree_navigation(tree: &mut dyn PointTree) -> Result<()> {
    let min_packed_value = tree.min_packed_value()?.to_vec();
    let max_packed_value = tree.max_packed_value()?.to_vec();
    let size = tree.size()?;
    if random().next_boolean() && tree.move_to_child()? {
        random_point_tree_navigation(tree)?;
        if random().next_boolean() && tree.move_to_sibling()? {
            random_point_tree_navigation(tree)?;
        }
        tree.move_to_parent()?;
    }
    // we always finish on the same node we started
    super::base_index_file_format_test_case::assert_array_equals(
        &min_packed_value,
        tree.min_packed_value()?,
    );
    super::base_index_file_format_test_case::assert_array_equals(
        &max_packed_value,
        tree.max_packed_value()?,
    );
    Assert::assert_equals(size, tree.size()?);
    Ok(())
}

/// `switchIndex(w, dir, saveW)` (`private`). Takes `w` by value: the
/// closed writer holds `dir`, released before `dir.close()`.
fn switch_index(
    w: RandomIndexWriter,
    dir: &mut TestDirectory,
    save_w: &RandomIndexWriter,
) -> Result<()> {
    if random().next_boolean() {
        // Add via readers:
        let r = w.get_reader()?;
        let outcome = (|| -> Result<()> {
            if random().next_boolean() {
                // Add via CodecReaders:
                let mut subs: Vec<Arc<dyn CodecReader>> = Vec::new();
                for context in Arc::clone(&r).leaves()? {
                    subs.push(
                        context
                            .leaf_reader()
                            .as_codec_reader()
                            .expect("a segment reader is a CodecReader"),
                    );
                }
                if *VERBOSE {
                    println!("TEST: now use addIndexes(CodecReader[]) to switch writers");
                }
                save_w.add_indexes_codec_readers(&subs)?;
            } else {
                if *VERBOSE {
                    println!(
                        "TEST: now use TestUtil.addIndexesSlowly(DirectoryReader[]) to switch writers"
                    );
                }
                TestUtil::add_indexes_slowly(save_w.w(), &[Arc::clone(&r)])?;
            }
            Ok(())
        })();
        let closed = r.close();
        outcome?;
        closed?;
    } else {
        // Add via directory:
        if *VERBOSE {
            println!("TEST: now use addIndexes(Directory[]) to switch writers");
        }
        w.close()?;
        save_w.add_indexes_dirs(&[dir.as_directory()])?;
    }
    w.close()?;
    drop(w);
    dir.close()
}

/// `getPointValues(size, docCount, estimatedPointCount)` (`private`).
fn get_point_values(size: i64, doc_count: i32, estimated_point_count: i64) -> FixedPointValues {
    FixedPointValues {
        size,
        doc_count,
        estimated_point_count,
    }
}
