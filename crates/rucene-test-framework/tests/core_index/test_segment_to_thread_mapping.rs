//! Port of `lucene/core/src/test/org/apache/lucene/index/TestSegmentToThreadMapping.java`.
//!
//! `IndexSearcher.slices(List, int, int, boolean)` (the static overload) is
//! `IndexSearcher::compute_slices` in Rucene, since Rust has no overloading.

use std::fmt;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use randomizedtesting::RandomizedTest;
use rucene::document::Document;
use rucene::error::Result;
use rucene::index::index_reader::{CacheHelper, IndexReaderCore, StoredFields};
use rucene::index::leaf_reader::EmptyTermVectors;
use rucene::index::{
    BinaryDocValues, ByteVectorValues, DocValuesSkipper, FieldInfos, FloatVectorValues,
    IndexReader, LeafMetaData, LeafReader, LeafReaderContext, NumericDocValues, PointValues,
    SortedDocValues, SortedNumericDocValues, SortedSetDocValues, TermVectors, Terms,
};
use rucene::search::index_searcher::{IndexSearcher, IndexSearcherOverrides, LeafSlice};
use rucene::search::knn::KnnCollector;
use rucene::search::task_executor::{Command, Executor, RejectedExecutionException};
use rucene::search::AcceptDocs;
use rucene::util::{Bits, IOUtils, Version};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, shuffle, test};

/// `TestSegmentToThreadMapping`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestSegmentToThreadMapping",
        vec![
            test("testSingleSlice", test_single_slice),
            test(
                "testSingleSliceWithPartitions",
                test_single_slice_with_partitions,
            ),
            test("testMaxSegmentsPerSlice", test_max_segments_per_slice),
            test(
                "testMaxSegmentsPerSliceWithPartitions",
                test_max_segments_per_slice_with_partitions,
            ),
            test("testSmallSegments", test_small_segments),
            test(
                "testSmallSegmentsWithPartitions",
                test_small_segments_with_partitions,
            ),
            test("testLargeSlices", test_large_slices),
            test(
                "testLargeSlicesWithPartitions",
                test_large_slices_with_partitions,
            ),
            test(
                "testSingleSegmentPartitions",
                test_single_segment_partitions,
            ),
            test(
                "testExtremeSegmentsPartitioning",
                test_extreme_segments_partitioning,
            ),
            test("testIntraSliceDocIDOrder", test_intra_slice_doc_id_order),
            test(
                "testIntraSliceDocIDOrderWithPartitions",
                test_intra_slice_doc_id_order_with_partitions,
            ),
            test("testRandom", test_random),
        ],
    )
}

/// The anonymous `LeafReader` of `dummyIndexReader(maxDoc)`.
struct DummyIndexReader {
    core: IndexReaderCore,
    max_doc: i32,
}

impl fmt::Debug for DummyIndexReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DummyIndexReader(maxDoc={})", self.max_doc)
    }
}

impl fmt::Display for DummyIndexReader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

/// The anonymous `StoredFields` of `dummyIndexReader`, whose `document` does
/// nothing.
#[derive(Debug)]
struct NoStoredFields;

impl StoredFields for NoStoredFields {
    fn document_with_visitor(
        &self,
        _doc_id: i32,
        _visitor: &mut dyn rucene::codecs::stub::StoredFieldVisitor,
    ) -> Result<()> {
        Ok(())
    }
}

impl LeafReader for DummyIndexReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }
    fn max_doc(&self) -> i32 {
        self.max_doc
    }
    fn num_docs(&self) -> i32 {
        self.max_doc
    }
    fn get_field_infos(&self) -> FieldInfos {
        FieldInfos::empty()
    }
    fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        None
    }
    fn terms(&self, _field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(None)
    }
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        Ok(Box::new(EmptyTermVectors))
    }
    fn get_numeric_doc_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }
    fn get_binary_doc_values(&self, _field: &str) -> Result<Option<Box<dyn BinaryDocValues>>> {
        Ok(None)
    }
    fn get_sorted_doc_values(&self, _field: &str) -> Result<Option<Box<dyn SortedDocValues>>> {
        Ok(None)
    }
    fn get_sorted_numeric_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedNumericDocValues>>> {
        Ok(None)
    }
    fn get_sorted_set_doc_values(
        &self,
        _field: &str,
    ) -> Result<Option<Box<dyn SortedSetDocValues>>> {
        Ok(None)
    }
    fn get_norm_values(&self, _field: &str) -> Result<Option<Box<dyn NumericDocValues>>> {
        Ok(None)
    }
    fn get_doc_values_skipper(&self, _field: &str) -> Result<Option<Box<dyn DocValuesSkipper>>> {
        Ok(None)
    }
    fn get_point_values(&self, _field: &str) -> Result<Option<Box<dyn PointValues>>> {
        Ok(None)
    }
    fn get_float_vector_values(&self, _field: &str) -> Result<Option<Box<dyn FloatVectorValues>>> {
        Ok(None)
    }
    fn get_byte_vector_values(&self, _field: &str) -> Result<Option<Box<dyn ByteVectorValues>>> {
        Ok(None)
    }
    fn search_nearest_vectors(
        &self,
        _field: &str,
        _target: &[f32],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }
    fn search_nearest_vectors_byte(
        &self,
        _field: &str,
        _target: &[u8],
        _collector: &mut dyn KnnCollector,
        _accept_docs: &mut dyn AcceptDocs,
    ) -> Result<()> {
        Ok(())
    }
    fn do_close(&self) -> Result<()> {
        Ok(())
    }
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        Ok(Box::new(NoStoredFields))
    }
    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }
    fn get_meta_data(&self) -> LeafMetaData {
        LeafMetaData::new(
            i32::from(Version::LATEST.major),
            Some(Version::LATEST),
            None,
            false,
        )
        .expect("the latest version is valid")
    }
    fn get_core_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        None
    }
}

/// `private static LeafReader dummyIndexReader(final int maxDoc)`.
fn dummy_index_reader(max_doc: i32) -> Arc<DummyIndexReader> {
    Arc::new(DummyIndexReader {
        core: IndexReaderCore::new(),
        max_doc,
    })
}

/// `new LeafReaderContext(reader)`: a top-level leaf context.
fn leaf_context(reader: Arc<DummyIndexReader>) -> Arc<LeafReaderContext> {
    let leaf: Arc<dyn LeafReader> = Arc::clone(&reader) as _;
    let index_reader: Arc<dyn IndexReader> = reader;
    Arc::new(LeafReaderContext::new(index_reader, leaf, None, 0, 0, 0, 0))
}

/// `private static List<LeafReaderContext> createLeafReaderContexts(int... maxDocs)`.
fn create_leaf_reader_contexts(max_docs: &[i32]) -> Vec<Arc<LeafReaderContext>> {
    let mut leaf_reader_contexts = Vec::new();
    for &max_doc in max_docs {
        leaf_reader_contexts.push(leaf_context(dummy_index_reader(max_doc)));
    }
    shuffle(&mut leaf_reader_contexts, &*random());
    leaf_reader_contexts
}

/// `IndexSearcher.slices(leaves, maxDocsPerSlice, maxSegmentsPerSlice,
/// allowSegmentPartitions)`.
fn slices(
    leaves: &[Arc<LeafReaderContext>],
    max_docs_per_slice: i32,
    max_segments_per_slice: i32,
    allow_segment_partitions: bool,
) -> Result<Vec<LeafSlice>> {
    IndexSearcher::compute_slices(
        leaves,
        max_docs_per_slice,
        max_segments_per_slice as usize,
        allow_segment_partitions,
    )
}

/// `assertEquals(partitions, slice.partitions.length)` and, when given,
/// `assertEquals(maxDocs, slice.getMaxDocs())`.
fn check_slice(slice: &LeafSlice, partitions: usize, max_docs: Option<i32>) {
    assert_equals(&partitions, &slice.partitions().len());
    if let Some(max_docs) = max_docs {
        assert_equals(&max_docs, &slice.get_max_docs());
    }
}

/// Port of `TestSegmentToThreadMapping.testSingleSlice`.
fn test_single_slice() -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[50_000, 30_000, 30_000, 30_000]);
    let result_slices = slices(
        &leaf_reader_contexts,
        250_000,
        RandomizedTest::random_int_between(4, 10),
        false,
    )?;
    assert_equals(&1, &result_slices.len());
    assert_equals(&4, &result_slices[0].partitions().len());
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testSingleSliceWithPartitions`.
fn test_single_slice_with_partitions() -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[50_000, 30_000, 30_000, 30_000]);
    let result_slices = slices(
        &leaf_reader_contexts,
        250_000,
        RandomizedTest::random_int_between(4, 10),
        true,
    )?;
    assert_equals(&1, &result_slices.len());
    assert_equals(&4, &result_slices[0].partitions().len());
    Ok(())
}

/// The three blocks of `testMaxSegmentsPerSlice` and its partitioned twin.
fn max_segments_per_slice(allow_segment_partitions: bool) -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[50_000, 30_000, 30_000, 30_000]);
    {
        let result_slices = slices(&leaf_reader_contexts, 250_000, 3, allow_segment_partitions)?;
        assert_equals(&2, &result_slices.len());
        check_slice(&result_slices[0], 3, Some(110_000));
        check_slice(&result_slices[1], 1, Some(30_000));
    }
    {
        let result_slices = slices(&leaf_reader_contexts, 250_000, 2, allow_segment_partitions)?;
        assert_equals(&2, &result_slices.len());
        check_slice(&result_slices[0], 2, Some(80_000));
        check_slice(&result_slices[1], 2, Some(60_000));
    }
    {
        let result_slices = slices(&leaf_reader_contexts, 250_000, 1, allow_segment_partitions)?;
        assert_equals(&4, &result_slices.len());
        check_slice(&result_slices[0], 1, Some(50_000));
        check_slice(&result_slices[1], 1, Some(30_000));
        check_slice(&result_slices[2], 1, Some(30_000));
        check_slice(&result_slices[3], 1, Some(30_000));
    }
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testMaxSegmentsPerSlice`.
fn test_max_segments_per_slice() -> Result<()> {
    max_segments_per_slice(false)
}

/// Port of `TestSegmentToThreadMapping.testMaxSegmentsPerSliceWithPartitions`.
fn test_max_segments_per_slice_with_partitions() -> Result<()> {
    max_segments_per_slice(true)
}

/// The two blocks of `testSmallSegments` and its partitioned twin.
fn small_segments(allow_segment_partitions: bool) -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[
        10_000, 10_000, 10_000, 10_000, 10_000, 10_000, 130_000, 130_000,
    ]);

    {
        let result_slices = slices(&leaf_reader_contexts, 250_000, 5, allow_segment_partitions)?;
        assert_equals(&3, &result_slices.len());

        check_slice(&result_slices[0], 2, Some(260_000));
        check_slice(&result_slices[1], 5, Some(50_000));
        check_slice(&result_slices[2], 1, Some(10_000));
    }
    {
        let result_slices = slices(&leaf_reader_contexts, 130_000, 5, allow_segment_partitions)?;
        assert_equals(&3, &result_slices.len());
        // this is odd, because we allow two segments in the same slice with both size ==
        // maxDocsPerSlice
        check_slice(&result_slices[0], 2, Some(260_000));
        check_slice(&result_slices[1], 5, Some(50_000));
        check_slice(&result_slices[2], 1, Some(10_000));
    }
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testSmallSegments`.
fn test_small_segments() -> Result<()> {
    small_segments(false)
}

/// Port of `TestSegmentToThreadMapping.testSmallSegmentsWithPartitions`.
fn test_small_segments_with_partitions() -> Result<()> {
    small_segments(true)
}

/// Port of `TestSegmentToThreadMapping.testLargeSlices`.
fn test_large_slices() -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[290_900, 170_000, 170_000, 170_000]);
    let result_slices = slices(&leaf_reader_contexts, 250_000, 5, false)?;

    assert_equals(&3, &result_slices.len());
    check_slice(&result_slices[0], 1, None);
    check_slice(&result_slices[1], 2, None);
    check_slice(&result_slices[2], 1, None);
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testLargeSlicesWithPartitions`.
fn test_large_slices_with_partitions() -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[290_900, 170_000, 170_000, 170_000]);
    let result_slices = slices(
        &leaf_reader_contexts,
        250_000,
        RandomizedTest::random_int_between(5, 10),
        true,
    )?;

    assert_equals(&4, &result_slices.len());
    check_slice(&result_slices[0], 1, Some(145_450));
    check_slice(&result_slices[1], 1, Some(145_450));
    check_slice(&result_slices[2], 2, Some(340_000));
    check_slice(&result_slices[3], 1, Some(170_000));
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testSingleSegmentPartitions`.
fn test_single_segment_partitions() -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[750_001]);
    let result_slices = slices(
        &leaf_reader_contexts,
        250_000,
        RandomizedTest::random_int_between(1, 10),
        true,
    )?;

    assert_equals(&4, &result_slices.len());
    check_slice(&result_slices[0], 1, Some(187_500));
    check_slice(&result_slices[1], 1, Some(187_500));
    check_slice(&result_slices[2], 1, Some(187_500));
    check_slice(&result_slices[3], 1, Some(187_501));
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testExtremeSegmentsPartitioning`.
fn test_extreme_segments_partitioning() -> Result<()> {
    let leaf_reader_contexts = create_leaf_reader_contexts(&[2, 5, 10]);
    let result_slices = slices(&leaf_reader_contexts, 1, 1, true)?;

    assert_equals(&12, &result_slices.len());
    let mut i = 0;
    for leaf_slice in &result_slices {
        let cur = i;
        i += 1;
        if cur > 4 {
            assert_equals(&1, &leaf_slice.get_max_docs());
        } else {
            assert_equals(&2, &leaf_slice.get_max_docs());
        }
        assert_equals(&1, &leaf_slice.partitions().len());
    }
    Ok(())
}

/// `command -> {}`: an `Executor` that drops every command.
struct NoOpExecutor;

impl Executor for NoOpExecutor {
    fn execute(&self, _command: Command) -> std::result::Result<(), RejectedExecutionException> {
        Ok(())
    }
}

/// The index both `testIntraSliceDocIDOrder*` tests build: two commits of two
/// empty documents, read through `w.getReader()`.
fn two_segment_reader(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<dyn IndexReader>> {
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    w.commit()?;
    w.add_document(&Document::new())?;
    w.add_document(&Document::new())?;
    w.commit()?;
    let r: Arc<dyn IndexReader> = w.get_reader()?;
    w.close()?;
    Ok(r)
}

/// The checks both `testIntraSliceDocIDOrder*` tests make over `s.getSlices()`.
fn check_intra_slice_doc_id_order(s: &IndexSearcher) -> Result<()> {
    let slices = s.get_slices()?;
    for leaf_slice in slices {
        let mut previous_doc_base = leaf_slice.partitions()[0].ctx.doc_base();

        for leaf_reader_context_partition in leaf_slice.partitions() {
            Assert::assert_true(previous_doc_base <= leaf_reader_context_partition.ctx.doc_base());
            previous_doc_base = leaf_reader_context_partition.ctx.doc_base();
        }
    }
    Ok(())
}

/// Port of `TestSegmentToThreadMapping.testIntraSliceDocIDOrder`.
fn test_intra_slice_doc_id_order() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let r = two_segment_reader(&dir)?;

    let s = IndexSearcher::with_executor(Arc::clone(&r), Some(Arc::new(NoOpExecutor)))?;
    // `assertNotNull(slices)`: `get_slices` returns a slice reference.
    check_intra_slice_doc_id_order(&s)?;
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// The anonymous `IndexSearcher` subclass of
/// `testIntraSliceDocIDOrderWithPartitions`, overriding `slices`.
#[derive(Debug)]
struct OnePerPartitionSlices;

impl IndexSearcherOverrides for OnePerPartitionSlices {
    fn slices(
        &self,
        _searcher: &IndexSearcher,
        leaves: &[Arc<LeafReaderContext>],
    ) -> Result<Vec<LeafSlice>> {
        // force partitioning of segment with max docs per slice set to 1: 1 doc per partition.
        slices(leaves, 1, 1, true)
    }
}

/// Port of `TestSegmentToThreadMapping.testIntraSliceDocIDOrderWithPartitions`.
fn test_intra_slice_doc_id_order_with_partitions() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let r = two_segment_reader(&dir)?;

    let mut s = IndexSearcher::with_executor(Arc::clone(&r), Some(Arc::new(NoOpExecutor)))?;
    s.set_overrides(Arc::new(OnePerPartitionSlices));
    check_intra_slice_doc_id_order(&s)?;
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestSegmentToThreadMapping.testRandom`.
fn test_random() -> Result<()> {
    let mut leaf_reader_contexts = Vec::new();
    let max = 500_000;
    let min = 10_000;
    let num_segments = 1 + random().next_int_bound(50);

    for _i in 0..num_segments {
        leaf_reader_contexts.push(leaf_context(dummy_index_reader(
            random().next_int_bound((max - min) + 1) + min,
        )));
    }
    let result_slices = slices(&leaf_reader_contexts, 250_000, 5, random().next_boolean())?;
    Assert::assert_true(!result_slices.is_empty());
    Ok(())
}
