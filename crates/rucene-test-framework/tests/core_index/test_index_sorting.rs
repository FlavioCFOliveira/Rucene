//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIndexSorting.java`.
//!
//! * `AssertingNeedsIndexSortCodec` is a `FilterCodec` whose points format
//!   wraps the default codec's; its two mutable fields are shared atomics, so
//!   the test sets them after the codec is installed, as Java does.
//! * The `PositionsTokenStream` of each `testRandom2` document is kept next to
//!   the document (the field holds the same `Arc`), and its id is set through
//!   that handle, as Java reaches it through `doc.getField("positions")`.
//! * `Document.add(field)` of a field that Java keeps and changes
//!   (`testIndexSortWithSparseField`, `testSortDocs`, `testWrongSortFieldType`)
//!   adds a copy, rebuilt with the current value (guide section 6).

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::collections::{HashMap, HashSet};
use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::analysis::{
    default_token_attribute_factory, Analyzer, AnalyzerState, GlobalReuseStrategy, TokenStream,
    TokenStreamComponents,
};
use rucene::codecs::{
    Codec, FilterCodec, MergeState, PointsFormat, PointsReader, PointsWriter, SegmentReadState,
    SegmentWriteState,
};
use rucene::document::{
    BinaryDocValuesField, BinaryPoint, Document, DoubleDocValuesField, Field, FieldType,
    FloatDocValuesField, IntPoint, NumericDocValuesField, NumericValue, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StoredField, StringField,
    TextField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::{self, IndexWriter};
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::indexing_chain::FieldInvertState;
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_ALL};
use rucene::index::terms::Term;
use rucene::index::{
    FieldInfo, IndexOptions, IndexReader, LeafReader, NumericDocValues, SortedDocValues,
};
use rucene::search::doc_id_set_iterator::NO_MORE_DOCS;
use rucene::search::similarities::{CollectionStatistics, SimScorer, Similarity, TermStatistics};
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::top_field_collector_manager::TopFieldCollectorManager;
use rucene::search::{
    MatchAllDocsQuery, Query, SortedNumericSelectorType, SortedNumericSortField,
    SortedSetSelectorType, SortedSetSortField, TermQuery,
};
use rucene::util::attribute::AttributeSource;
use rucene::util::java_collections::JavaIntHashMap;
use rucene::util::{BytesRef, FixedBitSet, IOUtils, NumericUtils};
use rucene_test_framework::analysis::attributes::{char_term_id, offset_id, payload_id, Att};
use rucene_test_framework::analysis::mock_tokenizer::MockTokenizer;
use rucene_test_framework::analysis::{tokenizer_components, MockAnalyzer};
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{
    assert_contains, assert_equals, assert_equals_delta, assert_not_null, assert_null, class,
    random, shuffle, test, verbose,
};
use crate::support::{copy_field, npe, CountDownLatch};

/// `TestIndexSorting`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestIndexSorting",
        vec![
            test("testNumericAlreadySorted", test_numeric_already_sorted),
            test("testStringAlreadySorted", test_string_already_sorted),
            test(
                "testMultiValuedNumericAlreadySorted",
                test_multi_valued_numeric_already_sorted,
            ),
            test(
                "testMultiValuedStringAlreadySorted",
                test_multi_valued_string_already_sorted,
            ),
            test("testBasicString", test_basic_string),
            test("testBasicMultiValuedString", test_basic_multi_valued_string),
            test("testMissingStringFirst", test_missing_string_first),
            test(
                "testMissingMultiValuedStringFirst",
                test_missing_multi_valued_string_first,
            ),
            test("testMissingStringLast", test_missing_string_last),
            test(
                "testMissingMultiValuedStringLast",
                test_missing_multi_valued_string_last,
            ),
            test("testBasicLong", test_basic_long),
            test("testBasicMultiValuedLong", test_basic_multi_valued_long),
            test("testMissingLongFirst", test_missing_long_first),
            test(
                "testMissingMultiValuedLongFirst",
                test_missing_multi_valued_long_first,
            ),
            test("testMissingLongLast", test_missing_long_last),
            test(
                "testMissingMultiValuedLongLast",
                test_missing_multi_valued_long_last,
            ),
            test("testBasicInt", test_basic_int),
            test("testBasicMultiValuedInt", test_basic_multi_valued_int),
            test("testMissingIntFirst", test_missing_int_first),
            test(
                "testMissingMultiValuedIntFirst",
                test_missing_multi_valued_int_first,
            ),
            test("testMissingIntLast", test_missing_int_last),
            test(
                "testMissingMultiValuedIntLast",
                test_missing_multi_valued_int_last,
            ),
            test("testBasicDouble", test_basic_double),
            test("testBasicMultiValuedDouble", test_basic_multi_valued_double),
            test("testMissingDoubleFirst", test_missing_double_first),
            test(
                "testMissingMultiValuedDoubleFirst",
                test_missing_multi_valued_double_first,
            ),
            test("testMissingDoubleLast", test_missing_double_last),
            test(
                "testMissingMultiValuedDoubleLast",
                test_missing_multi_valued_double_last,
            ),
            test("testBasicFloat", test_basic_float),
            test("testBasicMultiValuedFloat", test_basic_multi_valued_float),
            test("testMissingFloatFirst", test_missing_float_first),
            test(
                "testMissingMultiValuedFloatFirst",
                test_missing_multi_valued_float_first,
            ),
            test("testMissingFloatLast", test_missing_float_last),
            test(
                "testMissingMultiValuedFloatLast",
                test_missing_multi_valued_float_last,
            ),
            test("testRandom1", test_random1),
            test("testMultiValuedRandom1", test_multi_valued_random1),
            test("testConcurrentUpdates", test_concurrent_updates),
            test("testBadDVUpdate", test_bad_dv_update),
            test("testConcurrentDVUpdates", test_concurrent_dv_updates),
            test("testBadAddIndexes", test_bad_add_indexes),
            test("testAddIndexes", test_add_indexes),
            test(
                "testAddIndexesWithDeletions",
                test_add_indexes_with_deletions,
            ),
            test(
                "testAddIndexesWithDirectory",
                test_add_indexes_with_directory,
            ),
            test(
                "testAddIndexesWithDeletionsAndDirectory",
                test_add_indexes_with_deletions_and_directory,
            ),
            test("testBadSort", test_bad_sort),
            test("testIllegalChangeSort", test_illegal_change_sort),
            test("testRandom2", test_random2),
            test("testRandom3", test_random3),
            test("testTieBreak", test_tie_break),
            test(
                "testIndexSortWithSparseField",
                test_index_sort_with_sparse_field,
            ),
            test(
                "testIndexSortOnSparseField",
                test_index_sort_on_sparse_field,
            ),
            test("testWrongSortFieldType", test_wrong_sort_field_type),
            test("testDeleteByTermOrQuery", test_delete_by_term_or_query),
            test("testSortDocs", test_sort_docs),
            test("testSortDocsAndFreqs", test_sort_docs_and_freqs),
            test(
                "testSortDocsAndFreqsAndPositions",
                test_sort_docs_and_freqs_and_positions,
            ),
            test(
                "testSortDocsAndFreqsAndPositionsAndOffsets",
                test_sort_docs_and_freqs_and_positions_and_offsets,
            ),
            test(
                "testParentFieldNotConfigured",
                test_parent_field_not_configured,
            ),
            test(
                "testBlockContainsParentField",
                test_block_contains_parent_field,
            ),
            test("testIndexSortWithBlocks", test_index_sort_with_blocks),
            test(
                "testMixRandomDocumentsWithBlocks",
                test_mix_random_documents_with_blocks,
            ),
        ],
    )
}

// ---------------------------------------------------------------------------
// AssertingNeedsIndexSortCodec
// ---------------------------------------------------------------------------

/// The two fields of `AssertingNeedsIndexSortCodec`, shared with its points
/// writers.
#[derive(Debug, Default)]
struct NeedsIndexSortState {
    needs_index_sort: AtomicBool,
    num_calls: AtomicI32,
}

/// The anonymous `PointsFormat` of `AssertingNeedsIndexSortCodec.pointsFormat()`.
#[derive(Debug)]
struct AssertingNeedsIndexSortPointsFormat {
    delegate: Arc<dyn Codec>,
    state: Arc<NeedsIndexSortState>,
}

impl PointsFormat for AssertingNeedsIndexSortPointsFormat {
    fn name(&self) -> &str {
        self.delegate.points_format().name()
    }

    fn fields_writer(&self, state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        let writer = self.delegate.points_format().fields_writer(state)?;
        Ok(Box::new(AssertingNeedsIndexSortPointsWriter {
            writer,
            state: Arc::clone(&self.state),
        }))
    }

    fn fields_reader(&self, state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        self.delegate.points_format().fields_reader(state)
    }
}

/// The anonymous `PointsWriter` of `AssertingNeedsIndexSortCodec`.
#[derive(Debug)]
struct AssertingNeedsIndexSortPointsWriter {
    writer: Box<dyn PointsWriter>,
    state: Arc<NeedsIndexSortState>,
}

impl PointsWriter for AssertingNeedsIndexSortPointsWriter {
    fn merge(&mut self, merge_state: &MergeState) -> Result<()> {
        // For single segment merge we cannot infer if the segment is already sorted or not.
        if merge_state.doc_maps.len() > 1 {
            assert_equals(
                &self.state.needs_index_sort.load(Ordering::SeqCst),
                &merge_state.needs_index_sort,
            );
        }
        self.state.num_calls.fetch_add(1, Ordering::SeqCst);
        self.writer.merge(merge_state)
    }

    fn write_field(&mut self, field_info: &FieldInfo, values: &dyn PointsReader) -> Result<()> {
        self.writer.write_field(field_info, values)
    }

    fn finish(&mut self) -> Result<()> {
        self.writer.finish()
    }

    fn close(&mut self) -> Result<()> {
        self.writer.close()
    }
}

/// `new AssertingNeedsIndexSortCodec()`: the codec, and the state its
/// `needsIndexSort` and `numCalls` fields live in.
fn asserting_needs_index_sort_codec() -> (Arc<dyn Codec>, Arc<NeedsIndexSortState>) {
    let delegate = TestUtil::get_default_codec();
    let state = Arc::new(NeedsIndexSortState::default());
    let codec = FilterCodec::new(delegate.name(), Arc::clone(&delegate)).with_points_format(
        AssertingNeedsIndexSortPointsFormat {
            delegate,
            state: Arc::clone(&state),
        },
    );
    (Arc::new(codec), state)
}

/// `new Sort(fields...)`.
fn sort_of(fields: Vec<SortField>) -> Result<Sort> {
    Sort::new_fields(fields)
}

/// `new SortField(field, type)`.
fn sort_field(field: &str, field_type: SortFieldType) -> Result<SortField> {
    SortField::new(Some(field.to_string()), field_type)
}

/// `new SortField(field, type, reverse, missingValue)`.
fn sort_field_missing(
    field: &str,
    field_type: SortFieldType,
    reverse: bool,
    missing: Option<MissingValue>,
) -> Result<SortField> {
    SortField::new_with_missing(Some(field.to_string()), field_type, reverse, missing)
}

/// `new SortedNumericSortField(field, type, reverse, SortedNumericSelector.Type.MIN, missing)`.
fn sorted_numeric_min(
    field: &str,
    field_type: SortFieldType,
    reverse: bool,
    missing: Option<MissingValue>,
) -> Result<SortField> {
    Ok(SortedNumericSortField::with_missing_value(
        field,
        field_type,
        reverse,
        SortedNumericSelectorType::MIN,
        missing,
    )?
    .into_sort_field())
}

/// `new SortedSetSortField(field, reverse, SortedSetSelector.Type.MIN, missing)`.
fn sorted_set_min(field: &str, reverse: bool, missing: Option<MissingValue>) -> Result<SortField> {
    Ok(
        SortedSetSortField::with_missing_value(
            field,
            reverse,
            SortedSetSelectorType::MIN,
            missing,
        )?
        .into_sort_field(),
    )
}

/// `newBytesRef(s)`.
fn bytes_ref(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// `new NumericDocValuesField(name, value)`.
fn ndv(name: &str, value: i64) -> Box<NumericDocValuesField> {
    Box::new(NumericDocValuesField::new(name, value))
}

/// `new SortedNumericDocValuesField(name, value)`.
fn sndv(name: &str, value: i64) -> Box<SortedNumericDocValuesField> {
    Box::new(SortedNumericDocValuesField::new(name, value))
}

/// `new SortedSetDocValuesField(name, newBytesRef(value))`.
fn ssdv(name: &str, value: &str) -> Box<SortedSetDocValuesField> {
    Box::new(SortedSetDocValuesField::new(name, bytes_ref(value)))
}

/// `new SortedDocValuesField(name, newBytesRef(value))`.
fn sdv(name: &str, value: &str) -> Box<SortedDocValuesField> {
    Box::new(SortedDocValuesField::new(name, bytes_ref(value)))
}

/// `new StringField(name, value, stored)`.
fn string_field(name: &str, value: &str, stored: Store) -> Result<Box<StringField>> {
    Ok(Box::new(StringField::new(name, value.to_string(), stored)?))
}

/// `new NumericDocValuesField(name, value)` as the `Field` the doc-values
/// update methods take.
fn ndv_update(name: &str, value: i64) -> Result<Field> {
    Field::new_with_number(
        name,
        NumericValue::Long(value),
        NumericDocValuesField::type_().clone(),
    )
}

/// `new Term(field, text)`.
fn term(field: &str, text: &str) -> Term {
    Term::from_text(field, text)
}

/// `leaf.getNumericDocValues(field)`, dereferenced.
fn numeric(leaf: &Arc<dyn LeafReader>, field: &str) -> Result<Box<dyn NumericDocValues>> {
    Ok(leaf
        .get_numeric_doc_values(field)?
        .unwrap_or_else(|| npe("getNumericDocValues")))
}

/// `leaf.getSortedDocValues(field)`, dereferenced.
fn sorted(leaf: &Arc<dyn LeafReader>, field: &str) -> Result<Box<dyn SortedDocValues>> {
    Ok(leaf
        .get_sorted_doc_values(field)?
        .unwrap_or_else(|| npe("getSortedDocValues")))
}

/// `MultiDocValues.getNumericValues(reader, field)`, dereferenced.
fn multi_numeric(reader: &Arc<dyn IndexReader>, field: &str) -> Result<Box<dyn NumericDocValues>> {
    Ok(MultiDocValues::get_numeric_values(reader, field)?
        .unwrap_or_else(|| npe("getNumericValues")))
}

/// `assertEquals(doc, values.nextDoc()); assertEquals(value, values.longValue());`
/// for each pair.
fn assert_longs(values: &mut dyn NumericDocValues, expected: &[(i32, i64)]) -> Result<()> {
    for &(doc, value) in expected {
        assert_equals(&doc, &values.next_doc()?);
        assert_equals(&value, &values.long_value()?);
    }
    Ok(())
}

/// `assertEquals(doc, values.nextDoc()); assertEquals(value,
/// Double.longBitsToDouble(values.longValue()), 0.0);` for each pair.
fn assert_doubles(values: &mut dyn NumericDocValues, expected: &[(i32, f64)]) -> Result<()> {
    for &(doc, value) in expected {
        assert_equals(&doc, &values.next_doc()?);
        assert_equals_delta(value, f64::from_bits(values.long_value()? as u64), 0.0);
    }
    Ok(())
}

/// `assertEquals(doc, values.nextDoc()); assertEquals(value,
/// Float.intBitsToFloat((int) values.longValue()), 0.0f);` for each pair.
fn assert_floats(values: &mut dyn NumericDocValues, expected: &[(i32, f32)]) -> Result<()> {
    for &(doc, value) in expected {
        assert_equals(&doc, &values.next_doc()?);
        let actual = f32::from_bits(values.long_value()? as i32 as u32);
        assert_equals_delta(value as f64, actual as f64, 0.0);
    }
    Ok(())
}

/// `assertEquals(doc, values.nextDoc()); assertEquals(value,
/// values.lookupOrd(values.ordValue()).utf8ToString());` for each pair.
fn assert_ords(values: &mut dyn SortedDocValues, expected: &[(i32, &str)]) -> Result<()> {
    for &(doc, value) in expected {
        assert_equals(&doc, &values.next_doc()?);
        let ord = values.ord_value()?;
        assert_equals(
            &value.to_string(),
            &values.lookup_ord(ord)?.utf8_to_string()?,
        );
    }
    Ok(())
}

/// `assertNeedsIndexSortMerge(sortField, defaultValueConsumer, randomValueConsumer)`.
fn assert_needs_index_sort_merge(
    index_sort_field: SortField,
    default_value_consumer: &dyn Fn(&mut Document),
    random_value_consumer: &dyn Fn(&mut Document),
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let (codec, codec_state) = asserting_needs_index_sort_codec();
    iwc.set_codec(codec)?;
    let index_sort = sort_of(vec![
        index_sort_field,
        sort_field("id", SortFieldType::Int)?,
    ])?;
    iwc.set_index_sort(index_sort)?;
    let mut policy = LuceneTestCase::new_log_merge_policy();
    // make sure that merge factor is always > 2 and target search concurrency is no more than 1 to
    // avoid creating merges that are accidentally sorted
    policy.set_target_search_concurrency(1)?;
    if policy.get_merge_factor() <= 2 {
        policy.set_merge_factor(3)?;
    }
    iwc.set_merge_policy(Some(Arc::new(policy)))?;

    // add already sorted documents
    codec_state.num_calls.store(0, Ordering::SeqCst);
    codec_state.needs_index_sort.store(false, Ordering::SeqCst);
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let with_values = random().next_boolean();
    for i in 100..200 {
        let mut doc = Document::new();
        doc.add(string_field("id", &i.to_string(), Store::YES)?);
        doc.add(ndv("id", i));
        doc.add(Box::new(IntPoint::new("point", &[random().next_int()])?));
        if with_values {
            default_value_consumer(&mut doc);
        }
        w.add_document(&doc)?;
        if i % 10 == 0 {
            w.commit()?;
        }
    }
    let mut deleted_docs = HashSet::new();
    let num = random().next_int_bound(20);
    for _ in 0..num {
        let next_doc = random().next_int_bound(100);
        w.delete_documents_by_terms(vec![term("id", &next_doc.to_string())])?;
        deleted_docs.insert(next_doc);
    }
    w.commit()?;
    w.wait_for_merges()?;
    w.force_merge(1)?;
    Assert::assert_true(codec_state.num_calls.load(Ordering::SeqCst) > 0);

    // merge sort is needed
    w.delete_all()?;
    codec_state.num_calls.store(0, Ordering::SeqCst);
    codec_state.needs_index_sort.store(true, Ordering::SeqCst);
    for i in (0..=10).rev() {
        let mut doc = Document::new();
        doc.add(string_field("id", &i.to_string(), Store::YES)?);
        doc.add(ndv("id", i));
        doc.add(Box::new(IntPoint::new("point", &[random().next_int()])?));
        if with_values {
            default_value_consumer(&mut doc);
        }
        w.add_document(&doc)?;
        w.commit()?;
    }
    w.commit()?;
    w.wait_for_merges()?;
    w.force_merge(1)?;
    Assert::assert_true(codec_state.num_calls.load(Ordering::SeqCst) > 0);

    // segment sort is needed
    codec_state.needs_index_sort.store(true, Ordering::SeqCst);
    codec_state.num_calls.store(0, Ordering::SeqCst);
    for i in 201..300 {
        let mut doc = Document::new();
        doc.add(string_field("id", &i.to_string(), Store::YES)?);
        doc.add(ndv("id", i));
        doc.add(Box::new(IntPoint::new("point", &[random().next_int()])?));
        random_value_consumer(&mut doc);
        w.add_document(&doc)?;
        if i % 10 == 0 {
            w.commit()?;
        }
    }
    w.commit()?;
    w.wait_for_merges()?;
    w.force_merge(1)?;
    Assert::assert_true(codec_state.num_calls.load(Ordering::SeqCst) > 0);

    w.close()?;
    drop(w);
    dir.close()
}

/// Port of `TestIndexSorting.testNumericAlreadySorted`.
fn test_numeric_already_sorted() -> Result<()> {
    assert_needs_index_sort_merge(
        sort_field("foo", SortFieldType::Int)?,
        &|doc| doc.add(ndv("foo", 0)),
        &|doc| doc.add(ndv("foo", random().next_int() as i64)),
    )
}

/// Port of `TestIndexSorting.testStringAlreadySorted`.
fn test_string_already_sorted() -> Result<()> {
    assert_needs_index_sort_merge(
        sort_field("foo", SortFieldType::String)?,
        &|doc| doc.add(sdv("foo", "default")),
        &|doc| {
            doc.add(Box::new(SortedDocValuesField::new(
                "foo",
                TestUtil::random_binary_term(&*random()),
            )))
        },
    )
}

/// Port of `TestIndexSorting.testMultiValuedNumericAlreadySorted`.
fn test_multi_valued_numeric_already_sorted() -> Result<()> {
    assert_needs_index_sort_merge(
        SortedNumericSortField::new("foo", SortFieldType::Int)?.into_sort_field(),
        &|doc| {
            doc.add(sndv("foo", i32::MIN as i64));
            let num = random().next_int_bound(5);
            for _ in 0..num {
                doc.add(sndv("foo", random().next_int() as i64));
            }
        },
        &|doc| {
            let num = random().next_int_bound(5);
            for _ in 0..num {
                doc.add(sndv("foo", random().next_int() as i64));
            }
        },
    )
}

/// Port of `TestIndexSorting.testMultiValuedStringAlreadySorted`.
fn test_multi_valued_string_already_sorted() -> Result<()> {
    assert_needs_index_sort_merge(
        SortedSetSortField::new("foo", false)?.into_sort_field(),
        &|doc| {
            doc.add(ssdv("foo", ""));
            let num = random().next_int_bound(5);
            for _ in 0..num {
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "foo",
                    TestUtil::random_binary_term(&*random()),
                )));
            }
        },
        &|doc| {
            let num = random().next_int_bound(5);
            for _ in 0..num {
                doc.add(Box::new(SortedSetDocValuesField::new(
                    "foo",
                    TestUtil::random_binary_term(&*random()),
                )));
            }
        },
    )
}

/// The index the `testBasic*` and `testMissing*` methods build: three
/// documents in three segments, force-merged into one sorted segment.
struct ThreeDocIndex {
    dir: TestDirectory,
    w: Arc<IndexWriter>,
    r: Arc<dyn DirectoryReader>,
    leaf: Arc<dyn LeafReader>,
}

impl ThreeDocIndex {
    /// `r.close(); w.close(); dir.close();`.
    fn close(mut self) -> Result<()> {
        self.r.close()?;
        self.w.close()?;
        drop((self.w, self.leaf));
        self.dir.close()
    }
}

/// The body the `testBasic*` and `testMissing*` methods share: each document
/// is built, in order, right before it is added, as Java does.
fn index_three_docs(
    index_sort_field: SortField,
    docs: [&dyn Fn() -> Document; 3],
) -> Result<ThreeDocIndex> {
    let dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let index_sort = sort_of(vec![index_sort_field])?;
    iwc.set_index_sort(index_sort)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&docs[0]())?;
    // so we get more than one segment, so that forceMerge actually does merge, since we only get a
    // sorted segment by merging:
    w.commit()?;

    w.add_document(&docs[1]())?;
    w.commit()?;

    w.add_document(&docs[2]())?;
    w.force_merge(1)?;

    let r = directory_reader::open_with_writer(&*w)?;
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&r) as Arc<dyn IndexReader>)?;
    assert_equals(&3, &leaf.max_doc());
    Ok(ThreeDocIndex { dir, w, r, leaf })
}

/// A document with the given fields.
fn doc_with(fields: Vec<Box<dyn rucene::index::IndexableField>>) -> Document {
    let mut doc = Document::new();
    for field in fields {
        doc.add(field);
    }
    doc
}

/// Port of `TestIndexSorting.testBasicString`.
fn test_basic_string() -> Result<()> {
    let index = index_three_docs(
        sort_field("foo", SortFieldType::String)?,
        [
            &|| doc_with(vec![sdv("foo", "zzz")]),
            &|| doc_with(vec![sdv("foo", "aaa")]),
            &|| doc_with(vec![sdv("foo", "mmm")]),
        ],
    )?;
    let mut values = sorted(&index.leaf, "foo")?;
    assert_ords(&mut *values, &[(0, "aaa"), (1, "mmm"), (2, "zzz")])?;
    index.close()
}

/// Port of `TestIndexSorting.testBasicMultiValuedString`.
fn test_basic_multi_valued_string() -> Result<()> {
    let index = index_three_docs(
        SortedSetSortField::new("foo", false)?.into_sort_field(),
        [
            &|| doc_with(vec![ndv("id", 3), ssdv("foo", "zzz")]),
            &|| {
                doc_with(vec![
                    ndv("id", 1),
                    ssdv("foo", "aaa"),
                    ssdv("foo", "zzz"),
                    ssdv("foo", "bcg"),
                ])
            },
            &|| doc_with(vec![ndv("id", 2), ssdv("foo", "mmm"), ssdv("foo", "pppp")]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "id")?;
    assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
    index.close()
}

/// Port of `TestIndexSorting.testMissingStringFirst`.
fn test_missing_string_first() -> Result<()> {
    for reverse in [true, false] {
        let index = index_three_docs(
            sort_field_missing(
                "foo",
                SortFieldType::String,
                reverse,
                Some(MissingValue::StringFirst),
            )?,
            [
                &|| doc_with(vec![sdv("foo", "zzz")]),
                // missing
                &Document::new,
                &|| doc_with(vec![sdv("foo", "mmm")]),
            ],
        )?;
        let mut values = sorted(&index.leaf, "foo")?;
        if reverse {
            assert_ords(&mut *values, &[(0, "zzz"), (1, "mmm")])?;
        } else {
            // docID 0 is missing:
            assert_ords(&mut *values, &[(1, "mmm"), (2, "zzz")])?;
        }
        index.close()?;
    }
    Ok(())
}

/// Port of `TestIndexSorting.testMissingMultiValuedStringFirst`.
fn test_missing_multi_valued_string_first() -> Result<()> {
    for reverse in [true, false] {
        let index = index_three_docs(
            sorted_set_min("foo", reverse, Some(MissingValue::StringFirst))?,
            [
                &|| {
                    doc_with(vec![
                        ndv("id", 3),
                        ssdv("foo", "zzz"),
                        ssdv("foo", "zzza"),
                        ssdv("foo", "zzzd"),
                    ])
                },
                // missing
                &|| doc_with(vec![ndv("id", 1)]),
                &|| doc_with(vec![ndv("id", 2), ssdv("foo", "mmm"), ssdv("foo", "nnnn")]),
            ],
        )?;
        let mut values = numeric(&index.leaf, "id")?;
        if reverse {
            assert_longs(&mut *values, &[(0, 3), (1, 2), (2, 1)])?;
        } else {
            assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
        }
        index.close()?;
    }
    Ok(())
}

/// Port of `TestIndexSorting.testMissingStringLast`.
fn test_missing_string_last() -> Result<()> {
    for reverse in [true, false] {
        let index = index_three_docs(
            sort_field_missing(
                "foo",
                SortFieldType::String,
                reverse,
                Some(MissingValue::StringLast),
            )?,
            [
                &|| doc_with(vec![sdv("foo", "zzz")]),
                // missing
                &Document::new,
                &|| doc_with(vec![sdv("foo", "mmm")]),
            ],
        )?;
        let mut values = sorted(&index.leaf, "foo")?;
        if reverse {
            assert_ords(&mut *values, &[(1, "zzz"), (2, "mmm")])?;
        } else {
            assert_ords(&mut *values, &[(0, "mmm"), (1, "zzz")])?;
        }
        assert_equals(&NO_MORE_DOCS, &values.next_doc()?);
        index.close()?;
    }
    Ok(())
}

/// Port of `TestIndexSorting.testMissingMultiValuedStringLast`.
fn test_missing_multi_valued_string_last() -> Result<()> {
    for reverse in [true, false] {
        let index = index_three_docs(
            sorted_set_min("foo", reverse, Some(MissingValue::StringLast))?,
            [
                &|| doc_with(vec![ndv("id", 2), ssdv("foo", "zzz"), ssdv("foo", "zzzd")]),
                // missing
                &|| doc_with(vec![ndv("id", 3)]),
                &|| doc_with(vec![ndv("id", 1), ssdv("foo", "mmm"), ssdv("foo", "ppp")]),
            ],
        )?;
        let mut values = numeric(&index.leaf, "id")?;
        if reverse {
            assert_longs(&mut *values, &[(0, 3), (1, 2), (2, 1)])?;
        } else {
            assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
        }
        index.close()?;
    }
    Ok(())
}

/// Port of `TestIndexSorting.testBasicLong`.
fn test_basic_long() -> Result<()> {
    let index = index_three_docs(
        sort_field("foo", SortFieldType::Long)?,
        [
            &|| doc_with(vec![ndv("foo", 18)]),
            &|| doc_with(vec![ndv("foo", -1)]),
            &|| doc_with(vec![ndv("foo", 7)]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "foo")?;
    assert_longs(&mut *values, &[(0, -1), (1, 7), (2, 18)])?;
    index.close()
}

/// Port of `TestIndexSorting.testBasicMultiValuedLong`.
fn test_basic_multi_valued_long() -> Result<()> {
    let index = index_three_docs(
        SortedNumericSortField::new("foo", SortFieldType::Long)?.into_sort_field(),
        [
            &|| doc_with(vec![ndv("id", 3), sndv("foo", 18), sndv("foo", 35)]),
            &|| doc_with(vec![ndv("id", 1), sndv("foo", -1)]),
            &|| doc_with(vec![ndv("id", 2), sndv("foo", 7), sndv("foo", 22)]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "id")?;
    assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
    index.close()
}

/// The single-valued numeric `testMissing*` methods: `18`, a missing value,
/// then `7`, read back as `long_value` through `check`.
fn missing_numeric_test(
    sort_field: &dyn Fn(bool) -> Result<SortField>,
    value: &dyn Fn(&str, f64) -> Box<dyn rucene::index::IndexableField>,
    check: &dyn Fn(&mut dyn NumericDocValues, bool) -> Result<()>,
) -> Result<()> {
    for reverse in [true, false] {
        let index = index_three_docs(
            sort_field(reverse)?,
            [
                &|| doc_with(vec![value("foo", 18.0)]),
                // missing
                &Document::new,
                &|| doc_with(vec![value("foo", 7.0)]),
            ],
        )?;
        let mut values = numeric(&index.leaf, "foo")?;
        check(&mut *values, reverse)?;
        index.close()?;
    }
    Ok(())
}

/// The multi-valued numeric `testMissing*` methods: the `id` values read
/// back are `3, 2, 1` reversed and `1, 2, 3` otherwise.
fn missing_multi_valued_test(
    sort_field: &dyn Fn(bool) -> Result<SortField>,
    docs: [&dyn Fn() -> Document; 3],
) -> Result<()> {
    for reverse in [true, false] {
        let index = index_three_docs(sort_field(reverse)?, docs)?;
        let mut values = numeric(&index.leaf, "id")?;
        if reverse {
            assert_longs(&mut *values, &[(0, 3), (1, 2), (2, 1)])?;
        } else {
            assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
        }
        index.close()?;
    }
    Ok(())
}

/// `new NumericDocValuesField(name, (long) value)`.
fn long_dv(name: &str, value: f64) -> Box<dyn rucene::index::IndexableField> {
    ndv(name, value as i64)
}

/// `new DoubleDocValuesField(name, value)`.
fn double_dv(name: &str, value: f64) -> Box<dyn rucene::index::IndexableField> {
    Box::new(DoubleDocValuesField::new(name, value))
}

/// `new FloatDocValuesField(name, (float) value)`.
fn float_dv(name: &str, value: f64) -> Box<dyn rucene::index::IndexableField> {
    Box::new(FloatDocValuesField::new(name, value as f32))
}

/// Port of `TestIndexSorting.testMissingLongFirst`.
fn test_missing_long_first() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Long,
                reverse,
                Some(MissingValue::Long(i64::MIN)),
            )
        },
        &long_dv,
        &|values, reverse| {
            if reverse {
                assert_longs(values, &[(0, 18), (1, 7)])
            } else {
                // docID 0 has no value
                assert_longs(values, &[(1, 7), (2, 18)])
            }
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedLongFirst`.
fn test_missing_multi_valued_long_first() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Long,
                reverse,
                Some(MissingValue::Long(i64::MIN)),
            )
        },
        [
            &|| doc_with(vec![ndv("id", 3), sndv("foo", 18), sndv("foo", 27)]),
            // missing
            &|| doc_with(vec![ndv("id", 1)]),
            &|| doc_with(vec![ndv("id", 2), sndv("foo", 7), sndv("foo", 24)]),
        ],
    )
}

/// Port of `TestIndexSorting.testMissingLongLast`.
fn test_missing_long_last() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Long,
                reverse,
                Some(MissingValue::Long(i64::MAX)),
            )
        },
        &long_dv,
        &|values, reverse| {
            if reverse {
                // docID 0 is missing
                assert_longs(values, &[(1, 18), (2, 7)])?;
            } else {
                assert_longs(values, &[(0, 7), (1, 18)])?;
            }
            assert_equals(&NO_MORE_DOCS, &values.next_doc()?);
            Ok(())
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedLongLast`.
fn test_missing_multi_valued_long_last() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Long,
                reverse,
                Some(MissingValue::Long(i64::MAX)),
            )
        },
        [
            &|| doc_with(vec![ndv("id", 2), sndv("foo", 18), sndv("foo", 65)]),
            // missing
            &|| doc_with(vec![ndv("id", 3)]),
            &|| {
                doc_with(vec![
                    ndv("id", 1),
                    sndv("foo", 7),
                    sndv("foo", 34),
                    sndv("foo", 74),
                ])
            },
        ],
    )
}

/// Port of `TestIndexSorting.testBasicInt`.
fn test_basic_int() -> Result<()> {
    let index = index_three_docs(
        sort_field("foo", SortFieldType::Int)?,
        [
            &|| doc_with(vec![ndv("foo", 18)]),
            &|| doc_with(vec![ndv("foo", -1)]),
            &|| doc_with(vec![ndv("foo", 7)]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "foo")?;
    assert_longs(&mut *values, &[(0, -1), (1, 7), (2, 18)])?;
    index.close()
}

/// Port of `TestIndexSorting.testBasicMultiValuedInt`.
fn test_basic_multi_valued_int() -> Result<()> {
    let index = index_three_docs(
        SortedNumericSortField::new("foo", SortFieldType::Int)?.into_sort_field(),
        [
            &|| doc_with(vec![ndv("id", 3), sndv("foo", 18), sndv("foo", 34)]),
            &|| doc_with(vec![ndv("id", 1), sndv("foo", -1), sndv("foo", 34)]),
            &|| {
                doc_with(vec![
                    ndv("id", 2),
                    sndv("foo", 7),
                    sndv("foo", 22),
                    sndv("foo", 27),
                ])
            },
        ],
    )?;
    let mut values = numeric(&index.leaf, "id")?;
    assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
    index.close()
}

/// Port of `TestIndexSorting.testMissingIntFirst`.
fn test_missing_int_first() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Int,
                reverse,
                Some(MissingValue::Int(i32::MIN)),
            )
        },
        &long_dv,
        &|values, reverse| {
            if reverse {
                assert_longs(values, &[(0, 18), (1, 7)])
            } else {
                assert_longs(values, &[(1, 7), (2, 18)])
            }
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedIntFirst`.
fn test_missing_multi_valued_int_first() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Int,
                reverse,
                Some(MissingValue::Int(i32::MIN)),
            )
        },
        [
            &|| doc_with(vec![ndv("id", 3), sndv("foo", 18), sndv("foo", 187667)]),
            // missing
            &|| doc_with(vec![ndv("id", 1)]),
            &|| doc_with(vec![ndv("id", 2), sndv("foo", 7), sndv("foo", 34)]),
        ],
    )
}

/// Port of `TestIndexSorting.testMissingIntLast`.
fn test_missing_int_last() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Int,
                reverse,
                Some(MissingValue::Int(i32::MAX)),
            )
        },
        &long_dv,
        &|values, reverse| {
            if reverse {
                // docID 0 is missing
                assert_longs(values, &[(1, 18), (2, 7)])?;
            } else {
                assert_longs(values, &[(0, 7), (1, 18)])?;
            }
            assert_equals(&NO_MORE_DOCS, &values.next_doc()?);
            Ok(())
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedIntLast`.
fn test_missing_multi_valued_int_last() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Int,
                reverse,
                Some(MissingValue::Int(i32::MAX)),
            )
        },
        [
            &|| doc_with(vec![ndv("id", 2), sndv("foo", 18), sndv("foo", 6372)]),
            // missing
            &|| doc_with(vec![ndv("id", 3)]),
            &|| doc_with(vec![ndv("id", 1), sndv("foo", 7), sndv("foo", 8)]),
        ],
    )
}

/// `NumericUtils.doubleToSortableLong(value)`.
fn d2l(value: f64) -> i64 {
    NumericUtils::double_to_sortable_long(value)
}

/// `NumericUtils.floatToSortableInt(value)`.
fn f2i(value: f32) -> i64 {
    NumericUtils::float_to_sortable_int(value) as i64
}

/// Port of `TestIndexSorting.testBasicDouble`.
fn test_basic_double() -> Result<()> {
    let index = index_three_docs(
        sort_field("foo", SortFieldType::Double)?,
        [
            &|| doc_with(vec![double_dv("foo", 18.0)]),
            &|| doc_with(vec![double_dv("foo", -1.0)]),
            &|| doc_with(vec![double_dv("foo", 7.0)]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "foo")?;
    assert_doubles(&mut *values, &[(0, -1.0), (1, 7.0), (2, 18.0)])?;
    index.close()
}

/// Port of `TestIndexSorting.testBasicMultiValuedDouble`.
fn test_basic_multi_valued_double() -> Result<()> {
    let index = index_three_docs(
        SortedNumericSortField::new("foo", SortFieldType::Double)?.into_sort_field(),
        [
            &|| {
                doc_with(vec![
                    ndv("id", 3),
                    sndv("foo", d2l(7.54)),
                    sndv("foo", d2l(27.0)),
                ])
            },
            &|| {
                doc_with(vec![
                    ndv("id", 1),
                    sndv("foo", d2l(-1.0)),
                    sndv("foo", d2l(0.0)),
                ])
            },
            &|| {
                doc_with(vec![
                    ndv("id", 2),
                    sndv("foo", d2l(7.0)),
                    sndv("foo", d2l(7.67)),
                ])
            },
        ],
    )?;
    let mut values = numeric(&index.leaf, "id")?;
    assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
    index.close()
}

/// Port of `TestIndexSorting.testMissingDoubleFirst`.
fn test_missing_double_first() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Double,
                reverse,
                Some(MissingValue::Double(f64::NEG_INFINITY)),
            )
        },
        &double_dv,
        &|values, reverse| {
            if reverse {
                assert_doubles(values, &[(0, 18.0), (1, 7.0)])
            } else {
                assert_doubles(values, &[(1, 7.0), (2, 18.0)])
            }
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedDoubleFirst`.
fn test_missing_multi_valued_double_first() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Double,
                reverse,
                Some(MissingValue::Double(f64::NEG_INFINITY)),
            )
        },
        [
            &|| {
                doc_with(vec![
                    ndv("id", 3),
                    sndv("foo", d2l(18.0)),
                    sndv("foo", d2l(18.76)),
                ])
            },
            // missing
            &|| doc_with(vec![ndv("id", 1)]),
            &|| {
                doc_with(vec![
                    ndv("id", 2),
                    sndv("foo", d2l(7.0)),
                    sndv("foo", d2l(70.0)),
                ])
            },
        ],
    )
}

/// Port of `TestIndexSorting.testMissingDoubleLast`.
fn test_missing_double_last() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Double,
                reverse,
                Some(MissingValue::Double(f64::INFINITY)),
            )
        },
        &double_dv,
        &|values, reverse| {
            if reverse {
                // docID 0 is missing
                assert_doubles(values, &[(1, 18.0), (2, 7.0)])?;
            } else {
                assert_doubles(values, &[(0, 7.0), (1, 18.0)])?;
            }
            assert_equals(&NO_MORE_DOCS, &values.next_doc()?);
            Ok(())
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedDoubleLast`.
fn test_missing_multi_valued_double_last() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Double,
                reverse,
                Some(MissingValue::Double(f64::INFINITY)),
            )
        },
        [
            &|| {
                doc_with(vec![
                    ndv("id", 2),
                    sndv("foo", d2l(18.0)),
                    sndv("foo", d2l(8262.0)),
                ])
            },
            // missing
            &|| doc_with(vec![ndv("id", 3)]),
            &|| {
                doc_with(vec![
                    ndv("id", 1),
                    sndv("foo", d2l(7.0)),
                    sndv("foo", d2l(7.87)),
                ])
            },
        ],
    )
}

/// Port of `TestIndexSorting.testBasicFloat`.
fn test_basic_float() -> Result<()> {
    let index = index_three_docs(
        sort_field("foo", SortFieldType::Float)?,
        [
            &|| doc_with(vec![float_dv("foo", 18.0)]),
            &|| doc_with(vec![float_dv("foo", -1.0)]),
            &|| doc_with(vec![float_dv("foo", 7.0)]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "foo")?;
    assert_floats(&mut *values, &[(0, -1.0), (1, 7.0), (2, 18.0)])?;
    index.close()
}

/// Port of `TestIndexSorting.testBasicMultiValuedFloat`.
fn test_basic_multi_valued_float() -> Result<()> {
    let index = index_three_docs(
        SortedNumericSortField::new("foo", SortFieldType::Float)?.into_sort_field(),
        [
            &|| {
                doc_with(vec![
                    ndv("id", 3),
                    sndv("foo", f2i(18.0)),
                    sndv("foo", f2i(29.0)),
                ])
            },
            &|| {
                doc_with(vec![
                    ndv("id", 1),
                    sndv("foo", f2i(-1.0)),
                    sndv("foo", f2i(34.0)),
                ])
            },
            &|| doc_with(vec![ndv("id", 2), sndv("foo", f2i(7.0))]),
        ],
    )?;
    let mut values = numeric(&index.leaf, "id")?;
    assert_longs(&mut *values, &[(0, 1), (1, 2), (2, 3)])?;
    index.close()
}

/// Port of `TestIndexSorting.testMissingFloatFirst`.
fn test_missing_float_first() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Float,
                reverse,
                Some(MissingValue::Float(f32::NEG_INFINITY)),
            )
        },
        &float_dv,
        &|values, reverse| {
            if reverse {
                assert_floats(values, &[(0, 18.0), (1, 7.0)])
            } else {
                assert_floats(values, &[(1, 7.0), (2, 18.0)])
            }
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedFloatFirst`.
fn test_missing_multi_valued_float_first() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Float,
                reverse,
                Some(MissingValue::Float(f32::NEG_INFINITY)),
            )
        },
        [
            &|| {
                doc_with(vec![
                    ndv("id", 3),
                    sndv("foo", f2i(18.0)),
                    sndv("foo", f2i(726.0)),
                ])
            },
            // missing
            &|| doc_with(vec![ndv("id", 1)]),
            &|| {
                doc_with(vec![
                    ndv("id", 2),
                    sndv("foo", f2i(7.0)),
                    sndv("foo", f2i(18.0)),
                ])
            },
        ],
    )
}

/// Port of `TestIndexSorting.testMissingFloatLast`.
fn test_missing_float_last() -> Result<()> {
    missing_numeric_test(
        &|reverse| {
            sort_field_missing(
                "foo",
                SortFieldType::Float,
                reverse,
                Some(MissingValue::Float(f32::INFINITY)),
            )
        },
        &float_dv,
        &|values, reverse| {
            if reverse {
                // docID 0 is missing
                assert_floats(values, &[(1, 18.0), (2, 7.0)])?;
            } else {
                assert_floats(values, &[(0, 7.0), (1, 18.0)])?;
            }
            assert_equals(&NO_MORE_DOCS, &values.next_doc()?);
            Ok(())
        },
    )
}

/// Port of `TestIndexSorting.testMissingMultiValuedFloatLast`.
fn test_missing_multi_valued_float_last() -> Result<()> {
    missing_multi_valued_test(
        &|reverse| {
            sorted_numeric_min(
                "foo",
                SortFieldType::Float,
                reverse,
                Some(MissingValue::Float(f32::INFINITY)),
            )
        },
        [
            &|| {
                doc_with(vec![
                    ndv("id", 2),
                    sndv("foo", f2i(726.0)),
                    sndv("foo", f2i(18.0)),
                ])
            },
            // missing
            &|| doc_with(vec![ndv("id", 3)]),
            &|| {
                doc_with(vec![
                    ndv("id", 1),
                    sndv("foo", f2i(12.67)),
                    sndv("foo", f2i(7.0)),
                ])
            },
        ],
    )
}

/// `getOnlyLeafReader(reader)`.
fn only_leaf(reader: &Arc<dyn DirectoryReader>) -> Result<Arc<dyn LeafReader>> {
    LuceneTestCase::get_only_leaf_reader(Arc::clone(reader) as Arc<dyn IndexReader>)
}

/// `reader` as the `IndexReader` Java passes around.
fn ir(reader: &Arc<dyn DirectoryReader>) -> Arc<dyn IndexReader> {
    Arc::clone(reader) as Arc<dyn IndexReader>
}

/// The index-consistency check `testRandom1` and `testMultiValuedRandom1`
/// share.
fn check_random1_consistency(
    reader: &Arc<dyn DirectoryReader>,
    num_docs: i32,
    deleted: &FixedBitSet,
) -> Result<()> {
    let searcher = LuceneTestCase::new_searcher(Arc::clone(reader));
    let stored_fields = reader.stored_fields()?;
    for i in 0..num_docs {
        let term_query = Arc::new(TermQuery::new(term("id", &i.to_string())));
        let top_docs = searcher.search(term_query, 1)?;
        if deleted.get(i as usize) {
            assert_equals(&0, &top_docs.total_hits.value());
        } else {
            assert_equals(&1, &top_docs.total_hits.value());
            let mut values = multi_numeric(&ir(reader), "id")?;
            let doc = top_docs.score_docs[0].doc;
            assert_equals(&doc, &values.advance(doc)?);
            assert_equals(&(i as i64), &values.long_value()?);
            let document = stored_fields.document(doc)?;
            assert_equals(&Some(i.to_string()), &document.get("id"));
        }
    }
    Ok(())
}

/// The random indexing loop of `testRandom1` and `testMultiValuedRandom1`;
/// `values` adds the `foo` values of a document.
fn random1_index(
    w: &IndexWriter,
    num_docs: i32,
    deleted: &mut FixedBitSet,
    values: &dyn Fn(&mut Document),
) -> Result<()> {
    for i in 0..num_docs {
        let mut doc = Document::new();
        values(&mut doc);
        doc.add(string_field("id", &i.to_string(), Store::YES)?);
        doc.add(ndv("id", i as i64));
        w.add_document(&doc)?;
        if random().next_int_bound(5) == 0 {
            directory_reader::open_with_writer(w)?.close()?;
        } else if random().next_int_bound(30) == 0 {
            w.force_merge(2)?;
        } else if random().next_int_bound(4) == 0 {
            let id = TestUtil::next_int(&*random(), 0, i);
            deleted.set(id as usize);
            w.delete_documents_by_terms(vec![term("id", &id.to_string())])?;
        }
    }
    Ok(())
}

/// Port of `TestIndexSorting.testRandom1`.
fn test_random1() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let index_sort = sort_of(vec![sort_field("foo", SortFieldType::Long)?])?;
    iwc.set_index_sort(index_sort.clone())?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let num_docs = LuceneTestCase::at_least(200);
    let mut deleted = FixedBitSet::new(num_docs as usize);
    random1_index(&w, num_docs, &mut deleted, &|doc| {
        doc.add(ndv("foo", random().next_int_bound(20) as i64))
    })?;

    // Check that segments are sorted
    let reader = directory_reader::open_with_writer(&*w)?;
    for ctx in ir(&reader).leaves()? {
        let leaf_reader = ctx.leaf_reader();
        let leaf = leaf_reader.as_segment_reader().unwrap_or_else(|| {
            Throwable::new("java.lang.ClassCastException", "not a SegmentReader").throw()
        });
        let info = &leaf.get_segment_info().info;
        let source = info
            .get_diagnostics()
            .get(index_writer::SOURCE)
            .cloned()
            .unwrap_or_else(|| npe("diagnostics source"));
        match source.as_str() {
            index_writer::SOURCE_FLUSH | index_writer::SOURCE_MERGE => {
                assert_equals(&index_sort, info.index_sort());
                let mut values = numeric(&leaf_reader, "foo")?;
                let mut previous = i64::MIN;
                for i in 0..leaf_reader.max_doc() {
                    assert_equals(&i, &values.next_doc()?);
                    let value = values.long_value()?;
                    Assert::assert_true(value >= previous);
                    previous = value;
                }
            }
            _ => Assert::fail(""),
        }
    }

    // Now check that the index is consistent
    check_random1_consistency(&reader, num_docs, &deleted)?;

    reader.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// Port of `TestIndexSorting.testMultiValuedRandom1`.
fn test_multi_valued_random1() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let index_sort = sort_of(vec![SortedNumericSortField::new(
        "foo",
        SortFieldType::Long,
    )?
    .into_sort_field()])?;
    iwc.set_index_sort(index_sort)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let num_docs = LuceneTestCase::at_least(200);
    let mut deleted = FixedBitSet::new(num_docs as usize);
    random1_index(&w, num_docs, &mut deleted, &|doc| {
        let num = random().next_int_bound(10);
        for _ in 0..num {
            doc.add(sndv("foo", random().next_int_bound(2000) as i64));
        }
    })?;

    let reader = directory_reader::open_with_writer(&*w)?;
    // Now check that the index is consistent
    check_random1_consistency(&reader, num_docs, &deleted)?;

    reader.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// `catch (IOException | InterruptedException e) { throw new
/// RuntimeException(e); }` around a `Runnable` body.
fn rethrow_as_runtime(outcome: Result<()>) {
    if let Err(e) = outcome {
        let t = java_throwable(&e);
        if e.is_io_exception() || matches!(e, LuceneError::ThreadInterrupted(_)) {
            Throwable::from_cause("java.lang.RuntimeException", t).throw()
        }
        t.throw()
    }
}

/// `static class UpdateRunnable implements Runnable` (and, with `dv_updates`,
/// `DVUpdateRunnable`, which updates the `bar` doc-values field instead of
/// the whole document).
struct UpdateRunnable {
    num_docs: i32,
    random: JdkRandom,
    update_count: Arc<AtomicI32>,
    w: Arc<IndexWriter>,
    values: Arc<Mutex<HashMap<i32, i64>>>,
    latch: Arc<CountDownLatch>,
    dv_updates: bool,
}

impl UpdateRunnable {
    /// `run()`.
    fn run(&self) {
        rethrow_as_runtime((|| -> Result<()> {
            self.latch.await_();
            while self.update_count.fetch_sub(1, Ordering::SeqCst) - 1 >= 0 {
                let id = self.random.next_int_bound(self.num_docs);
                let value = self.random.next_int_bound(20) as i64;
                if self.dv_updates {
                    let mut values = self.values.lock().unwrap_or_else(|e| e.into_inner());
                    self.w.update_doc_values(
                        term("id", &id.to_string()),
                        &[ndv_update("bar", value)?],
                    )?;
                    values.insert(id, value);
                } else {
                    let mut doc = Document::new();
                    doc.add(string_field("id", &id.to_string(), Store::NO)?);
                    doc.add(ndv("foo", value));

                    let mut values = self.values.lock().unwrap_or_else(|e| e.into_inner());
                    self.w.update_document(term("id", &id.to_string()), &doc)?;
                    values.insert(id, value);
                }

                match self.random.next_int_bound(10) {
                    0 | 1 => {
                        // reopen
                        directory_reader::open_with_writer(&*self.w)?.close()?;
                    }
                    2 => {
                        self.w.force_merge(3)?;
                    }
                    _ => {}
                }
            }
            Ok(())
        })())
    }
}

/// `new Thread(runnable)` for each runnable, started, released by `latch`,
/// and joined.
fn run_update_threads(
    num_docs: i32,
    w: &Arc<IndexWriter>,
    values: &Arc<Mutex<HashMap<i32, i64>>>,
    dv_updates: bool,
) -> Result<()> {
    let update_count = Arc::new(AtomicI32::new(LuceneTestCase::at_least(1000)));
    let latch = Arc::new(CountDownLatch::new(1));
    let mut threads = Vec::with_capacity(2);
    for _ in 0..2 {
        let runnable = UpdateRunnable {
            num_docs,
            random: JdkRandom::new(random().next_long()),
            update_count: Arc::clone(&update_count),
            w: Arc::clone(w),
            values: Arc::clone(values),
            latch: Arc::clone(&latch),
            dv_updates,
        };
        threads.push(Thread::unnamed(move || runnable.run()));
    }
    for thread in &threads {
        thread.start();
    }
    latch.count_down();
    for thread in &threads {
        if let Err(payload) = thread.join() {
            resume_unwind(payload);
        }
    }
    Ok(())
}

/// There is tricky logic to resolve deletes that happened while merging
/// Port of `TestIndexSorting.testConcurrentUpdates`.
fn test_concurrent_updates() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let values = Arc::new(Mutex::new(HashMap::new()));

    let num_docs = LuceneTestCase::at_least(100);
    run_update_threads(num_docs, &w, &values, false)?;
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let values = values.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for i in 0..num_docs {
        let top_docs = searcher.search(Arc::new(TermQuery::new(term("id", &i.to_string()))), 1)?;
        match values.get(&i) {
            None => assert_equals(&0, &top_docs.total_hits.value()),
            Some(&value) => {
                assert_equals(&1, &top_docs.total_hits.value());
                let mut dvs = multi_numeric(&ir(&reader), "foo")?;
                let doc_id = top_docs.score_docs[0].doc;
                assert_equals(&doc_id, &dvs.advance(doc_id)?);
                assert_equals(&value, &dvs.long_value()?);
            }
        }
    }
    reader.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// docvalues fields involved in the index sort cannot be updated
/// Port of `TestIndexSorting.testBadDVUpdate`.
fn test_bad_dv_update() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new_with_bytes(
        "id",
        bytes_ref("0"),
        Store::NO,
    )?));
    doc.add(ndv("foo", random().next_int() as i64));
    w.add_document(&doc)?;
    w.commit()?;
    let message =
        iae_message_of(|| w.update_doc_values(term("id", "0"), &[ndv_update("foo", -1)?]));
    assert_equals(
        &message,
        &"cannot update docvalues field involved in the index sort, field=foo, sort=<long: \"foo\">"
            .to_string(),
    );
    let message = iae_message_of(|| w.update_numeric_doc_value(term("id", "0"), "foo", -1));
    assert_equals(
        &message,
        &"cannot update docvalues field involved in the index sort, field=foo, sort=<long: \"foo\">"
            .to_string(),
    );
    w.close()?;
    drop(w);
    dir.close()
}

/// `expectThrows(IllegalArgumentException.class, runnable).getMessage()`.
fn iae_message_of<T>(runnable: impl FnOnce() -> Result<T>) -> String {
    crate::core_support::iae_message(runnable)
}

/// There is tricky logic to resolve dv updates that happened while merging
/// Port of `TestIndexSorting.testConcurrentDVUpdates`.
fn test_concurrent_dv_updates() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let values = Arc::new(Mutex::new(HashMap::new()));

    let num_docs = LuceneTestCase::at_least(100);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(string_field("id", &i.to_string(), Store::NO)?);
        doc.add(ndv("foo", random().next_int() as i64));
        doc.add(ndv("bar", -1));
        w.add_document(&doc)?;
        values
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(i, -1i64);
    }
    run_update_threads(num_docs, &w, &values, true)?;
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let values = values.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for i in 0..num_docs {
        let top_docs = searcher.search(Arc::new(TermQuery::new(term("id", &i.to_string()))), 1)?;
        assert_equals(&1, &top_docs.total_hits.value());
        let mut dvs = multi_numeric(&ir(&reader), "bar")?;
        let hit_doc = top_docs.score_docs[0].doc;
        assert_equals(&hit_doc, &dvs.advance(hit_doc)?);
        let value = *values.get(&i).unwrap_or_else(|| npe("values.get(i)"));
        assert_equals(&value, &dvs.long_value()?);
    }
    reader.close()?;
    w.close()?;
    drop(w);
    dir.close()
}

/// `(CodecReader) reader.leaves().get(i).reader()` for every leaf.
fn codec_readers(
    reader: &Arc<dyn IndexReader>,
) -> Result<Vec<Arc<dyn rucene::index::codec_reader::CodecReader>>> {
    Ok(Arc::clone(reader)
        .leaves()?
        .iter()
        .map(|ctx| {
            ctx.leaf_reader().as_codec_reader().unwrap_or_else(|| {
                Throwable::new("java.lang.ClassCastException", "not a CodecReader").throw()
            })
        })
        .collect())
}

/// Port of `TestIndexSorting.testBadAddIndexes`.
fn test_bad_add_indexes() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let index_sort = sort_of(vec![sort_field("foo", SortFieldType::Long)?])?;
    let iwc1 = LuceneTestCase::new_index_writer_config();
    iwc1.set_index_sort(index_sort)?;
    let w = IndexWriter::new(dir.as_directory(), iwc1)?;
    w.add_document(&Document::new())?;
    let index_sorts = vec![
        None,
        Some(sort_of(vec![sort_field("bar", SortFieldType::Long)?])?),
    ];
    for sort in index_sorts {
        let mut dir2 = LuceneTestCase::new_directory();
        let iwc2 = LuceneTestCase::new_index_writer_config();
        if let Some(sort) = sort {
            iwc2.set_index_sort(sort)?;
        }
        let w2 = IndexWriter::new(dir2.as_directory(), iwc2)?;
        w2.add_document(&Document::new())?;
        let reader = ir(&directory_reader::open_with_writer(&*w2)?);
        w2.close()?;
        let d2 = dir2.as_directory();
        let message = iae_message_of(|| w.add_indexes_dirs(&[Arc::clone(&d2)]));
        assert_contains(&message, "cannot change index sort");
        let readers = codec_readers(&reader)?;
        let message = iae_message_of(|| w.add_indexes_codec_readers(&readers));
        assert_contains(&message, "cannot change index sort");

        drop(readers);
        reader.close()?;
        drop((w2, d2, reader));
        dir2.close()?;
    }
    w.close()?;
    drop(w);
    dir.close()
}

/// Port of `TestIndexSorting.testAddIndexes(boolean, boolean)`.
fn do_test_add_indexes(with_deletes: bool, use_readers: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc1 = LuceneTestCase::new_index_writer_config();
    let use_parent = LuceneTestCase::rarely();
    if use_parent {
        iwc1.set_parent_field(Some("___parent".to_string()));
    }
    let index_sort = sort_of(vec![
        sort_field("foo", SortFieldType::Long)?,
        sort_field("bar", SortFieldType::Long)?,
    ])?;
    iwc1.set_index_sort(index_sort.clone())?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc1)?;
    let num_docs = LuceneTestCase::at_least(100);
    for i in 0..num_docs {
        let mut doc = Document::new();
        doc.add(string_field("id", &i.to_string(), Store::NO)?);
        doc.add(ndv("foo", random().next_int_bound(20) as i64));
        doc.add(ndv("bar", random().next_int_bound(20) as i64));
        w.add_document(&doc)?;
    }
    if with_deletes {
        let mut i = random().next_int_bound(5);
        while i < num_docs {
            w.delete_documents(term("id", &i.to_string()))?;
            i += TestUtil::next_int(&*random(), 1, 5);
        }
    }
    if random().next_boolean() {
        w.force_merge(1)?;
    }
    let reader_dr = w.get_reader()?;
    let reader = ir(&reader_dr);
    w.close()?;
    drop(w);

    let mut dir2 = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    if random().next_boolean() {
        // test congruent index sort
        iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Long)?])?)?;
    } else {
        iwc.set_index_sort(index_sort)?;
    }
    if use_parent {
        iwc.set_parent_field(Some("___parent".to_string()));
    }
    let w2 = IndexWriter::new(dir2.as_directory(), iwc)?;

    if use_readers {
        let readers = codec_readers(&reader)?;
        w2.add_indexes_codec_readers(&readers)?;
    } else {
        w2.add_indexes_dirs(&[dir.as_directory()])?;
    }
    let reader2_dr = directory_reader::open_with_writer(&*w2)?;
    let reader2 = ir(&reader2_dr);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader_dr));
    let searcher2 = LuceneTestCase::new_searcher(Arc::clone(&reader2_dr));
    for i in 0..num_docs {
        let query: Arc<dyn Query> = Arc::new(TermQuery::new(term("id", &i.to_string())));
        let top_docs = searcher.search(Arc::clone(&query), 1)?;
        let top_docs2 = searcher2.search(Arc::clone(&query), 1)?;
        assert_equals(&top_docs.total_hits.value(), &top_docs2.total_hits.value());
        if top_docs.total_hits.value() == 1 {
            let mut dvs1 = multi_numeric(&reader, "foo")?;
            let hit_doc1 = top_docs.score_docs[0].doc;
            assert_equals(&hit_doc1, &dvs1.advance(hit_doc1)?);
            let value1 = dvs1.long_value()?;
            let mut dvs2 = multi_numeric(&reader2, "foo")?;
            let hit_doc2 = top_docs2.score_docs[0].doc;
            assert_equals(&hit_doc2, &dvs2.advance(hit_doc2)?);
            let value2 = dvs2.long_value()?;
            assert_equals(&value1, &value2);
        }
    }

    drop((searcher, searcher2, reader_dr, reader2_dr));
    IOUtils::close([
        IOUtils::closeable(|| reader.close()),
        IOUtils::closeable(|| reader2.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// Port of `TestIndexSorting.testAddIndexes`.
fn test_add_indexes() -> Result<()> {
    do_test_add_indexes(false, true)
}

/// Port of `TestIndexSorting.testAddIndexesWithDeletions`.
fn test_add_indexes_with_deletions() -> Result<()> {
    do_test_add_indexes(true, true)
}

/// Port of `TestIndexSorting.testAddIndexesWithDirectory`.
fn test_add_indexes_with_directory() -> Result<()> {
    do_test_add_indexes(false, false)
}

/// Port of `TestIndexSorting.testAddIndexesWithDeletionsAndDirectory`.
fn test_add_indexes_with_deletions_and_directory() -> Result<()> {
    do_test_add_indexes(true, false)
}

/// Port of `TestIndexSorting.testBadSort`.
fn test_bad_sort() -> Result<()> {
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let message = iae_message_of(|| iwc.set_index_sort(Sort::new()).map(|_| ()));
    assert_equals(
        &"Cannot sort index with sort field <score>".to_string(),
        &message,
    );
    Ok(())
}

/// you can't change the index sort on an existing index:
/// Port of `TestIndexSorting.testIllegalChangeSort`.
fn test_illegal_change_sort() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let d = dir.as_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(Arc::clone(&d), iwc)?;
    w.add_document(&Document::new())?;
    directory_reader::open_with_writer(&*w)?.close()?;
    w.add_document(&Document::new())?;
    w.force_merge(1)?;
    w.close()?;
    drop(w);

    let iwc2 = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc2.set_index_sort(sort_of(vec![sort_field("bar", SortFieldType::Long)?])?)?;
    let message = iae_message_of(|| IndexWriter::new(Arc::clone(&d), iwc2));
    assert_contains(&message, "cannot change previous indexSort=<long: \"foo\">");
    assert_contains(&message, "to new indexSort=<long: \"bar\">");
    drop(d);
    dir.close()
}

/// `static final class NormsSimilarity extends Similarity`.
#[derive(Debug)]
struct NormsSimilarity {
    inner: Arc<dyn Similarity>,
}

impl Similarity for NormsSimilarity {
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexSorting$NormsSimilarity"
    }

    fn compute_norm(&self, state: &FieldInvertState) -> Result<i64> {
        if state.name() == "norms" {
            Ok(state.length() as i64)
        } else {
            self.inner.compute_norm(state)
        }
    }

    fn scorer<'a>(
        &'a self,
        boost: f32,
        collection_stats: &CollectionStatistics,
        term_stats: &[TermStatistics],
    ) -> Box<dyn SimScorer + 'a> {
        self.inner.scorer(boost, collection_stats, term_stats)
    }
}

/// `static final class PositionsTokenStream extends TokenStream`.
#[derive(Debug)]
struct PositionsTokenStream {
    source: AttributeSource,
    term: Att,
    payload: Att,
    offset: Att,
    pos: i32,
    off: i32,
}

impl PositionsTokenStream {
    /// `new PositionsTokenStream()`.
    fn new() -> Result<Self> {
        let mut source = AttributeSource::new_with_factory(default_token_attribute_factory());
        let term = Att::add(&mut source, char_term_id())?;
        let payload = Att::add(&mut source, payload_id())?;
        let offset = Att::add(&mut source, offset_id())?;
        Ok(PositionsTokenStream {
            source,
            term,
            payload,
            offset,
            pos: 0,
            off: 0,
        })
    }

    /// `setId(id)`.
    fn set_id(&mut self, id: i32) {
        self.pos = id / 10 + 1;
        self.off = 0;
    }
}

impl TokenStream for PositionsTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.pos == 0 {
            return Ok(false);
        }

        self.source.clear_attributes();
        self.term.char_term_mut().append_string(Some("#all#"));
        self.payload
            .payload_mut()
            .set_payload(Some(bytes_ref(&self.pos.to_string())));
        self.offset.offset_mut().set_offset(self.off, self.off);
        self.pos -= 1;
        self.off += 1;
        Ok(true)
    }

    /// `TokenStream.reset()`, which `PositionsTokenStream` does not
    /// override: it does nothing.
    fn reset(&mut self) -> Result<()> {
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// The anonymous `Analyzer` of `testRandom2`: a `MockTokenizer()`.
#[derive(Debug)]
struct MockTokenizerAnalyzer {
    state: AnalyzerState,
}

impl Analyzer for MockTokenizerAnalyzer {
    fn analyzer_state(&self) -> &AnalyzerState {
        &self.state
    }

    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.index.TestIndexSorting$1"
    }

    fn create_components(&self, _field_name: &str) -> Result<TokenStreamComponents> {
        let tokenizer = MockTokenizer::whitespace_lower_case()?;
        Ok(tokenizer_components(Arc::new(AtomicRefCell::new(
            tokenizer,
        ))))
    }
}

/// `((PositionsTokenStream) ((Field) doc.getField("positions"))
/// .tokenStreamValue()).setId(Integer.parseInt(doc.get("id")))`.
fn set_positions_id(doc: &Document, positions: &Arc<AtomicRefCell<PositionsTokenStream>>) {
    let id: i32 = doc
        .get("id")
        .unwrap_or_else(|| npe("doc.get(\"id\")"))
        .parse()
        .unwrap_or_else(|_| Throwable::new("java.lang.NumberFormatException", "id").throw());
    positions.borrow_mut().set_id(id);
}

/// Port of `TestIndexSorting.testRandom2`.
fn test_random2() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(100);

    let positions_type = FieldType::new_from(TextField::type_not_stored());
    positions_type.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)?;
    positions_type.freeze();

    let term_vectors_type = FieldType::new_from(TextField::type_not_stored());
    term_vectors_type.set_store_term_vectors(true)?;
    term_vectors_type.freeze();

    let a: Arc<dyn Analyzer> = Arc::new(MockTokenizerAnalyzer {
        state: AnalyzerState::new(Box::new(GlobalReuseStrategy)),
    });

    let mut docs: Vec<(Document, Arc<AtomicRefCell<PositionsTokenStream>>)> = Vec::new();
    for i in 0..num_docs {
        let id = i * 10;
        let mut doc = Document::new();
        doc.add(string_field("id", &id.to_string(), Store::YES)?);
        doc.add(string_field("docs", "#all#", Store::NO)?);
        let positions = Arc::new(AtomicRefCell::new(PositionsTokenStream::new()?));
        positions.borrow_mut().set_id(id);
        doc.add(Box::new(Field::new_with_token_stream(
            "positions",
            Arc::clone(&positions) as Arc<AtomicRefCell<dyn TokenStream>>,
            positions_type.clone(),
        )?));
        doc.add(ndv("numeric", id as i64));
        let value = (0..id)
            .map(|_| id.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        let norms = TextField::new("norms", value, Store::NO)?;
        doc.add(Box::new(norms));
        doc.add(Box::new(BinaryDocValuesField::new(
            "binary",
            bytes_ref(&id.to_string()),
        )));
        doc.add(sdv("sorted", &id.to_string()));
        doc.add(ssdv("multi_valued_string", &id.to_string()));
        doc.add(ssdv("multi_valued_string", &(id + 1).to_string()));
        doc.add(sndv("multi_valued_numeric", id as i64));
        doc.add(sndv("multi_valued_numeric", id as i64 + 1));
        doc.add(Box::new(Field::new(
            "term_vectors",
            id.to_string(),
            term_vectors_type.clone(),
        )?));
        let mut bytes = [0u8; 4];
        NumericUtils::int_to_sortable_bytes(id, &mut bytes, 0);
        doc.add(Box::new(BinaryPoint::new(
            "points",
            &[BytesRef::new(bytes.to_vec())],
        )?));
        docs.push((doc, positions));
    }

    // Must use the same seed for both RandomIndexWriters so they behave identically
    let seed = random().next_long();

    // We add document already in ID order for the first writer:
    let mut dir1 = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());

    let random1 = JdkRandom::new(seed);
    let iwc1 = LuceneTestCase::new_index_writer_config_with(&random1, Some(Arc::clone(&a)));
    let sim1 = iwc1.similarity();
    iwc1.set_similarity(Some(Arc::new(NormsSimilarity { inner: sim1 })))?; // for testing norms field
                                                                           // preserve docIDs
    iwc1.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    if verbose() {
        println!("TEST: now index pre-sorted");
    }
    let w1 = RandomIndexWriter::new_with_config(&random1, dir1.as_directory(), iwc1)?;
    for (doc, positions) in &docs {
        set_positions_id(doc, positions);
        w1.add_document(doc)?;
    }

    // We shuffle documents, but set index sort, for the second writer:
    let mut dir2 = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());

    let random2 = JdkRandom::new(seed);
    let iwc2 = LuceneTestCase::new_index_writer_config_with(&random2, Some(Arc::clone(&a)));
    let sim2 = iwc2.similarity();
    iwc2.set_similarity(Some(Arc::new(NormsSimilarity { inner: sim2 })))?; // for testing norms field

    let sort = sort_of(vec![sort_field("numeric", SortFieldType::Int)?])?;
    iwc2.set_index_sort(sort.clone())?;

    shuffle(&mut docs, &*random());
    if verbose() {
        println!("TEST: now index with index-time sorting");
    }
    let w2 = RandomIndexWriter::new_with_config(&random2, dir2.as_directory(), iwc2)?;
    let mut count = 0;
    let commit_at_count = TestUtil::next_int(&*random(), 1, num_docs - 1);
    for (doc, positions) in &docs {
        set_positions_id(doc, positions);
        let this_count = count;
        count += 1;
        if this_count == commit_at_count {
            // Ensure forceMerge really does merge
            w2.commit()?;
        }
        w2.add_document(doc)?;
    }
    if verbose() {
        println!("TEST: now force merge");
    }
    w2.force_merge(1)?;

    let r1 = w1.get_reader()?;
    let r2 = w2.get_reader()?;
    if verbose() {
        println!("TEST: now compare r1={r1} r2={r2}");
    }
    assert_equals(&Some(&sort), &only_leaf(&r2)?.get_meta_data().sort());
    LuceneTestCase::assert_reader_equals(
        "left: sorted by hand; right: sorted by Lucene",
        &ir(&r1),
        &ir(&r2),
    );
    IOUtils::close([
        IOUtils::closeable(|| w1.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// `private static final class RandomDoc`.
struct RandomDoc {
    int_value: i32,
    int_values: Vec<i32>,
    long_value: i64,
    long_values: Vec<i64>,
    float_value: f32,
    float_values: Vec<f32>,
    double_value: f64,
    double_values: Vec<f64>,
    bytes_value: Vec<u8>,
    bytes_values: Vec<Vec<u8>>,
}

impl RandomDoc {
    /// `new RandomDoc(id)`.
    fn new(_id: i32) -> Self {
        let int_value = random().next_int();
        let long_value = random().next_long();
        let float_value = random().next_float();
        let double_value = random().next_double();
        let mut bytes_value = vec![0u8; TestUtil::next_int(&*random(), 1, 50) as usize];
        random().next_bytes(&mut bytes_value);

        let num_values = random().next_int_bound(10) as usize;
        let mut int_values = vec![0; num_values];
        let mut long_values = vec![0; num_values];
        let mut float_values = vec![0.0; num_values];
        let mut double_values = vec![0.0; num_values];
        let mut bytes_values = vec![Vec::new(); num_values];
        for i in 0..num_values {
            int_values[i] = random().next_int();
            long_values[i] = random().next_long();
            float_values[i] = random().next_float();
            double_values[i] = random().next_double();
            bytes_values[i] = vec![0u8; TestUtil::next_int(&*random(), 1, 50) as usize];
            // Java fills `bytesValue` here, not `bytesValues[i]`.
            random().next_bytes(&mut bytes_value);
        }
        RandomDoc {
            int_value,
            int_values,
            long_value,
            long_values,
            float_value,
            float_values,
            double_value,
            double_values,
            bytes_value,
            bytes_values,
        }
    }
}

/// `randomIndexSortField()`.
fn random_index_sort_field() -> Result<SortField> {
    let reversed = random().next_boolean();
    match random().next_int_bound(10) {
        0 => sort_field_missing(
            "int",
            SortFieldType::Int,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Int(random().next_int()))
            } else {
                None
            },
        ),
        1 => sorted_numeric_min(
            "multi_valued_int",
            SortFieldType::Int,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Int(random().next_int()))
            } else {
                None
            },
        ),
        2 => sort_field_missing(
            "long",
            SortFieldType::Long,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Long(random().next_long()))
            } else {
                None
            },
        ),
        3 => sorted_numeric_min(
            "multi_valued_long",
            SortFieldType::Long,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Long(random().next_long()))
            } else {
                None
            },
        ),
        4 => sort_field_missing(
            "float",
            SortFieldType::Float,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Float(random().next_float()))
            } else {
                None
            },
        ),
        5 => sorted_numeric_min(
            "multi_valued_float",
            SortFieldType::Float,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Float(random().next_float()))
            } else {
                None
            },
        ),
        6 => sort_field_missing(
            "double",
            SortFieldType::Double,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Double(random().next_double()))
            } else {
                None
            },
        ),
        7 => sorted_numeric_min(
            "multi_valued_double",
            SortFieldType::Double,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::Double(random().next_double()))
            } else {
                None
            },
        ),
        8 => sort_field_missing(
            "bytes",
            SortFieldType::String,
            reversed,
            if random().next_boolean() {
                Some(MissingValue::StringLast)
            } else {
                None
            },
        ),
        9 => sorted_set_min(
            "multi_valued_bytes",
            reversed,
            if random().next_boolean() {
                Some(MissingValue::StringLast)
            } else {
                None
            },
        ),
        _ => Assert::fail(""),
    }
}

/// `randomSort()`.
fn random_sort() -> Result<Sort> {
    // at least 2
    let num_fields = TestUtil::next_int(&*random(), 2, 4);
    let mut sort_fields = Vec::with_capacity(num_fields as usize);
    for _ in 0..num_fields - 1 {
        sort_fields.push(random_index_sort_field()?);
    }

    // tie-break by id:
    sort_fields.push(sort_field("id", SortFieldType::Int)?);

    sort_of(sort_fields)
}

/// pits index time sorting against query time sorting
/// Port of `TestIndexSorting.testRandom3`.
fn test_random3() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    let mut docs = Vec::new();

    let sort = random_sort()?;
    if verbose() {
        println!("TEST: numDocs={num_docs} use sort={sort}");
    }

    // no index sorting, all search-time sorting:
    let mut dir1 = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let iwc1 = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    let w1 = IndexWriter::new(dir1.as_directory(), iwc1)?;

    // use index sorting:
    let mut dir2 = LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir());
    let iwc2 = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc2.set_index_sort(sort.clone())?;
    let w2 = IndexWriter::new(dir2.as_directory(), iwc2)?;

    // `Set<Integer> toDelete = new HashSet<>()`, iterated in Java's order.
    let mut to_delete: JavaIntHashMap<()> = JavaIntHashMap::new();

    let delete_chance = random().next_double();

    for id in 0..num_docs {
        let doc_values = RandomDoc::new(id);
        if verbose() {
            println!("TEST: doc id={id}");
            println!("  int={}", doc_values.int_value);
            println!("  long={}", doc_values.long_value);
            println!("  float={}", doc_values.float_value);
            println!("  double={}", doc_values.double_value);
            println!(
                "  bytes={}",
                LuceneTestCase::new_bytes_ref_bytes(&doc_values.bytes_value)
            );
            println!("  mvf={:?}", doc_values.float_values);
        }

        let mut doc = Document::new();
        doc.add(string_field("id", &id.to_string(), Store::YES)?);
        doc.add(ndv("id", id as i64));
        doc.add(ndv("int", doc_values.int_value as i64));
        doc.add(ndv("long", doc_values.long_value));
        doc.add(Box::new(DoubleDocValuesField::new(
            "double",
            doc_values.double_value,
        )));
        doc.add(Box::new(FloatDocValuesField::new(
            "float",
            doc_values.float_value,
        )));
        doc.add(Box::new(SortedDocValuesField::new(
            "bytes",
            LuceneTestCase::new_bytes_ref_bytes(&doc_values.bytes_value),
        )));

        for &value in &doc_values.int_values {
            doc.add(sndv("multi_valued_int", value as i64));
        }

        for &value in &doc_values.long_values {
            doc.add(sndv("multi_valued_long", value));
        }

        for &value in &doc_values.float_values {
            doc.add(sndv("multi_valued_float", f2i(value)));
        }

        for &value in &doc_values.double_values {
            doc.add(sndv("multi_valued_double", d2l(value)));
        }

        for value in &doc_values.bytes_values {
            doc.add(Box::new(SortedSetDocValuesField::new(
                "multi_valued_bytes",
                LuceneTestCase::new_bytes_ref_bytes(value),
            )));
        }

        w1.add_document(&doc)?;
        w2.add_document(&doc)?;
        if random().next_double() < delete_chance {
            to_delete.insert(id, ());
        }
        docs.push(doc_values);
    }
    for (id, _) in to_delete.iter() {
        w1.delete_documents_by_terms(vec![term("id", &id.to_string())])?;
        w2.delete_documents_by_terms(vec![term("id", &id.to_string())])?;
    }
    let r1 = directory_reader::open_with_writer(&*w1)?;
    let s1 = LuceneTestCase::new_searcher(Arc::clone(&r1));

    if random().next_boolean() {
        let max_segment_count = TestUtil::next_int(&*random(), 1, 5);
        if verbose() {
            println!("TEST: now forceMerge({max_segment_count})");
        }
        w2.force_merge(max_segment_count)?;
    }

    let r2 = directory_reader::open_with_writer(&*w2)?;
    let s2 = LuceneTestCase::new_searcher(Arc::clone(&r2));

    for iter in 0..100 {
        let num_hits = TestUtil::next_int(&*random(), 1, num_docs);
        if verbose() {
            println!("TEST: iter={iter} numHits={num_hits}");
        }

        let hits1 = s1.search_with_manager(
            Arc::new(MatchAllDocsQuery::instance()),
            &TopFieldCollectorManager::with_num_hits(sort.clone(), num_hits, i32::MAX)?,
        )?;
        let hits2 = s2.search_with_manager(
            Arc::new(MatchAllDocsQuery::instance()),
            &TopFieldCollectorManager::with_num_hits(sort.clone(), num_hits, 1)?,
        )?;

        if verbose() {
            println!(
                "  topDocs query-time sort: totalHits={}",
                hits1.total_hits.value()
            );
            for score_doc in &hits1.score_docs {
                println!("    {}", score_doc.score_doc.doc);
            }
            println!(
                "  topDocs index-time sort: totalHits={}",
                hits2.total_hits.value()
            );
            for score_doc in &hits2.score_docs {
                println!("    {}", score_doc.score_doc.doc);
            }
        }

        assert_equals(&hits2.score_docs.len(), &hits1.score_docs.len());
        let stored_fields1 = r1.stored_fields()?;
        let stored_fields2 = r2.stored_fields()?;
        for i in 0..hits2.score_docs.len() {
            let hit1 = &hits1.score_docs[i];
            let hit2 = &hits2.score_docs[i];
            assert_equals(
                &stored_fields1.document(hit1.score_doc.doc)?.get("id"),
                &stored_fields2.document(hit2.score_doc.doc)?.get("id"),
            );
            assert_equals(&hit1.fields, &hit2.fields);
        }
    }

    drop((s1, s2, docs));
    IOUtils::close([
        IOUtils::closeable(|| r1.close()),
        IOUtils::closeable(|| r2.close()),
        IOUtils::closeable(|| w1.close()),
        IOUtils::closeable(|| w2.close()),
        IOUtils::closeable(|| dir1.close()),
        IOUtils::closeable(|| dir2.close()),
    ])
}

/// Port of `TestIndexSorting.testTieBreak`.
fn test_tie_break() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::String)?])?)?;
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for id in 0..1000 {
        let mut doc = Document::new();
        doc.add(Box::new(StoredField::new_number(
            "id",
            NumericValue::Int(id),
        )?));
        let value = if id < 500 { "bar2" } else { "bar1" };
        doc.add(sdv("foo", value));
        w.add_document(&doc)?;
        if id == 500 {
            w.commit()?;
        }
    }
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let stored_fields = r.stored_fields()?;
    for doc_id in 0..1000 {
        let expected_id = if doc_id < 500 {
            500 + doc_id
        } else {
            doc_id - 500
        };
        let document = stored_fields.document(doc_id)?;
        let value = document
            .get_field("id")
            .unwrap_or_else(|| npe("getField(\"id\")"))
            .numeric_value()
            .unwrap_or_else(|| npe("numericValue()"));
        // `Number.intValue()`.
        let value = match value {
            NumericValue::Int(v) => v,
            NumericValue::Long(v) => v as i32,
            NumericValue::Float(v) => v as i32,
            NumericValue::Double(v) => v as i32,
        };
        assert_equals(&expected_id, &value);
    }
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexSorting.testIndexSortWithSparseField`.
fn test_index_sort_with_sparse_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let sort_field =
        SortField::new_reverse(Some("dense_int".to_string()), SortFieldType::Int, true)?;
    iwc.set_index_sort(sort_of(vec![sort_field])?)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    let mut text_field = LuceneTestCase::new_text_field("sparse_text", "", Store::NO);
    for i in 0..128 {
        let mut doc = Document::new();
        doc.add(ndv("dense_int", i));
        if i < 64 {
            doc.add(ndv("sparse_int", i));
            doc.add(Box::new(BinaryDocValuesField::new(
                "sparse_binary",
                bytes_ref(&i.to_string()),
            )));
            text_field.set_string_value("foo".to_string())?;
            doc.add(Box::new(copy_field(&text_field)?));
        }
        w.add_document(&doc)?;
    }
    w.commit()?;
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let leaves = ir(&r).leaves()?;
    assert_equals(&1, &leaves.len());
    let leaf_reader = leaves[0].leaf_reader();

    let mut dense_values = numeric(&leaf_reader, "dense_int")?;
    let mut sparse_values = numeric(&leaf_reader, "sparse_int")?;
    let mut sparse_binary_values = leaf_reader
        .get_binary_doc_values("sparse_binary")?
        .unwrap_or_else(|| npe("getBinaryDocValues"));
    let mut norms_values = leaf_reader
        .get_norm_values("sparse_text")?
        .unwrap_or_else(|| npe("getNormValues"));
    for doc_id in 0..128 {
        Assert::assert_true(dense_values.advance_exact(doc_id)?);
        assert_equals(&(127 - doc_id), &(dense_values.long_value()? as i32));
        if doc_id >= 64 {
            Assert::assert_true(dense_values.advance_exact(doc_id)?);
            Assert::assert_true(sparse_values.advance_exact(doc_id)?);
            Assert::assert_true(sparse_binary_values.advance_exact(doc_id)?);
            Assert::assert_true(norms_values.advance_exact(doc_id)?);
            assert_equals(&1, &norms_values.long_value()?);
            assert_equals(&(127 - doc_id), &(sparse_values.long_value()? as i32));
            assert_equals(
                &bytes_ref(&(127 - doc_id).to_string()),
                &sparse_binary_values.binary_value()?,
            );
        } else {
            Assert::assert_false(sparse_binary_values.advance_exact(doc_id)?);
            Assert::assert_false(sparse_values.advance_exact(doc_id)?);
            Assert::assert_false(norms_values.advance_exact(doc_id)?);
        }
    }
    drop(leaves);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexSorting.testIndexSortOnSparseField`.
fn test_index_sort_on_sparse_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let sort_field = sort_field_missing(
        "sparse",
        SortFieldType::Int,
        false,
        Some(MissingValue::Int(i32::MIN)),
    )?;
    iwc.set_index_sort(sort_of(vec![sort_field])?)?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for i in 0..128 {
        let mut doc = Document::new();
        if i < 64 {
            doc.add(ndv("sparse", i));
        }
        w.add_document(&doc)?;
    }
    w.commit()?;
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let leaves = ir(&r).leaves()?;
    assert_equals(&1, &leaves.len());
    let leaf_reader = leaves[0].leaf_reader();
    let mut sparse_values = numeric(&leaf_reader, "sparse")?;
    for doc_id in 0..128 {
        if doc_id >= 64 {
            Assert::assert_true(sparse_values.advance_exact(doc_id)?);
            assert_equals(&(doc_id - 64), &(sparse_values.long_value()? as i32));
        } else {
            Assert::assert_false(sparse_values.advance_exact(doc_id)?);
        }
    }
    drop(leaves);
    IOUtils::close([
        IOUtils::closeable(|| r.close()),
        IOUtils::closeable(|| w.close()),
        IOUtils::closeable(|| dir.close()),
    ])
}

/// Port of `TestIndexSorting.testWrongSortFieldType`.
fn test_wrong_sort_field_type() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    // The four fields of `dvs`; each use adds a fresh copy of the same field
    // (the two `newBytesRef` draws happen here, once, as in Java).
    let sorted_value = bytes_ref("");
    let sorted_set_value = bytes_ref("");
    let dvs = |j: usize| -> Box<dyn rucene::index::IndexableField> {
        match j {
            0 => Box::new(SortedDocValuesField::new("field", sorted_value.clone())),
            1 => Box::new(SortedSetDocValuesField::new(
                "field",
                sorted_set_value.clone(),
            )),
            2 => Box::new(NumericDocValuesField::new("field", 42)),
            _ => Box::new(SortedNumericDocValuesField::new("field", 42)),
        }
    };

    let sort_fields = vec![
        sort_field("field", SortFieldType::String)?,
        SortedSetSortField::new("field", false)?.into_sort_field(),
        sort_field("field", SortFieldType::Int)?,
        SortedNumericSortField::new("field", SortFieldType::Int)?.into_sort_field(),
    ];

    for i in 0..sort_fields.len() {
        for j in 0..4 {
            if i == j {
                continue;
            }
            let index_sort = sort_of(vec![sort_fields[i].clone()])?;
            let iwc =
                IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
            iwc.set_index_sort(index_sort)?;
            let w = IndexWriter::new(dir.as_directory(), iwc)?;
            let mut doc = Document::new();
            doc.add(dvs(j));
            let message = iae_message_of(|| w.add_document(&doc));
            assert_contains(&message, "expected field [field] to be ");
            doc.clear();
            doc.add(dvs(i));
            w.add_document(&doc)?;
            doc.add(dvs(j));
            let message = iae_message_of(|| w.add_document(&doc));
            assert_equals(
                &format!(
                    "Inconsistency of field data structures across documents for field [field] of doc [2]. doc values type: expected '{:?}', but it has '{:?}'.",
                    dvs(i).field_type().doc_values_type(),
                    dvs(j).field_type().doc_values_type()
                ),
                &message,
            );
            w.rollback()?;
            IOUtils::close([IOUtils::closeable(|| w.close())])?;
        }
    }
    IOUtils::close([IOUtils::closeable(|| dir.close())])
}

/// Port of `TestIndexSorting.testDeleteByTermOrQuery`.
fn test_delete_by_term_or_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_index_sort(sort_of(vec![sort_field("numeric", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let mut doc = Document::new();
    let num_docs = random().next_int_bound(2000) + 5;
    let mut expected_values = vec![0i64; num_docs as usize];

    for i in 0..num_docs as usize {
        expected_values[i] = random().next_int_bound(i32::MAX) as i64;
        doc.clear();
        doc.add(string_field("id", &i.to_string(), Store::YES)?);
        doc.add(ndv("numeric", expected_values[i]));
        w.add_document(&doc)?;
    }
    let num_deleted = random().next_int_bound(num_docs) + 1;
    for _ in 0..num_deleted {
        let id_to_delete = random().next_int_bound(num_docs);
        if random().next_boolean() {
            w.delete_documents_by_queries(vec![Arc::new(TermQuery::new(term(
                "id",
                &id_to_delete.to_string(),
            )))])?;
        } else {
            w.delete_documents_by_terms(vec![term("id", &id_to_delete.to_string())])?;
        }

        expected_values[id_to_delete as usize] = -(random().next_int_bound(i32::MAX) as i64); // force a reordering
        doc.clear();
        doc.add(string_field("id", &id_to_delete.to_string(), Store::YES)?);
        doc.add(ndv("numeric", expected_values[id_to_delete as usize]));
        w.add_document(&doc)?;
    }

    let mut doc_count = 0;
    {
        let reader = ir(&directory_reader::open_with_writer(&*w)?);
        for leaf_ctx in Arc::clone(&reader).leaves()? {
            let leaf = leaf_ctx.leaf_reader();
            let live_docs = leaf.get_live_docs();
            let Some(mut values) = leaf.get_numeric_doc_values("numeric")? else {
                continue;
            };
            let stored_fields = leaf.stored_fields()?;
            for id in 0..leaf.max_doc() {
                if let Some(live_docs) = &live_docs {
                    if !live_docs.get(id as usize) {
                        continue;
                    }
                }
                if !values.advance_exact(id)? {
                    continue;
                }
                let global_id: usize = stored_fields
                    .document(id)?
                    .get_field("id")
                    .unwrap_or_else(|| npe("getField(\"id\")"))
                    .string_value()
                    .unwrap_or_else(|| npe("stringValue()"))
                    .parse()
                    .unwrap_or_else(|_| {
                        Throwable::new("java.lang.NumberFormatException", "id").throw()
                    });
                Assert::assert_true(values.advance_exact(id)?);
                assert_equals(&expected_values[global_id], &values.long_value()?);
                doc_count += 1;
            }
        }
        assert_equals(&doc_count, &num_docs);
        reader.close()?;
    }
    w.close()?;
    drop(w);
    dir.close()
}

/// `new BytesRef(s)`.
fn utf8(s: &str) -> BytesRef {
    BytesRef::new(s.as_bytes().to_vec())
}

/// Port of `TestIndexSorting.testSortDocs`.
fn test_sort_docs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_index_sort(sort_of(vec![sort_field("sort", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    // `sort.setLongValue(v); field.setStringValue(s); w.addDocument(doc);`:
    // the document is rebuilt with the current values.
    for (sort, field) in [(0, "a"), (1, "b"), (-1, "a"), (2, "a"), (3, "b")] {
        let mut doc = Document::new();
        doc.add(ndv("sort", sort));
        doc.add(string_field("field", field, Store::NO)?);
        w.add_document(&doc)?;
    }
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let leaf_reader = only_leaf(&reader)?;
    let mut field_terms = leaf_reader
        .terms("field")?
        .unwrap_or_else(|| npe("terms(\"field\")"))
        .iterator()?;
    assert_equals(&Some(utf8("a")), &field_terms.next()?);
    let mut postings = field_terms.postings(None, POSTINGS_ENUM_ALL)?;
    assert_equals(&0, &postings.next_doc()?);
    assert_equals(&1, &postings.next_doc()?);
    assert_equals(&3, &postings.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    assert_equals(&Some(utf8("b")), &field_terms.next()?);
    let mut postings = field_terms.postings(Some(postings), POSTINGS_ENUM_ALL)?;
    assert_equals(&2, &postings.next_doc()?);
    assert_equals(&4, &postings.next_doc()?);
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    assert_null(&field_terms.next()?);
    reader.close()?;
    drop(w);
    dir.close()
}

/// The five documents of `testSortDocsAndFreqs*`: a `sort` value and the
/// `field` values, over `ft`.
fn add_sort_docs(w: &IndexWriter, ft: &FieldType, docs: &[(i64, &[&str])]) -> Result<()> {
    for &(sort, values) in docs {
        let mut doc = Document::new();
        doc.add(ndv("sort", sort));
        for value in values {
            doc.add(Box::new(Field::new(
                "field",
                value.to_string(),
                ft.clone(),
            )?));
        }
        w.add_document(&doc)?;
    }
    Ok(())
}

/// `assertEquals(doc, postings.nextDoc()); assertEquals(freq,
/// postings.freq());`.
fn assert_doc_freq(postings: &mut dyn PostingsEnum, doc: i32, freq: i32) -> Result<()> {
    assert_equals(&doc, &postings.next_doc()?);
    assert_equals(&freq, &postings.freq()?);
    Ok(())
}

/// `assertEquals(position, postings.nextPosition())`, and, with offsets,
/// `assertEquals(start, postings.startOffset()); assertEquals(end,
/// postings.endOffset());`.
fn assert_position(
    postings: &mut dyn PostingsEnum,
    position: i32,
    offsets: Option<(i32, i32)>,
) -> Result<()> {
    assert_equals(&position, &postings.next_position()?);
    if let Some((start, end)) = offsets {
        assert_equals(&start, &postings.start_offset());
        assert_equals(&end, &postings.end_offset());
    }
    Ok(())
}

/// Port of `TestIndexSorting.testSortDocsAndFreqs`.
fn test_sort_docs_and_freqs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config();
    config.set_index_sort(sort_of(vec![sort_field("sort", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let ft = FieldType::new();
    ft.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
    ft.set_tokenized(false)?;
    ft.freeze();
    add_sort_docs(
        &w,
        &ft,
        &[
            (0, &["a", "a"]),
            (1, &["b"]),
            (-1, &["a", "a", "a"]),
            (2, &["a"]),
            (3, &["b", "b", "b"]),
        ],
    )?;
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let leaf_reader = only_leaf(&reader)?;
    let mut field_terms = leaf_reader
        .terms("field")?
        .unwrap_or_else(|| npe("terms(\"field\")"))
        .iterator()?;
    assert_equals(&Some(utf8("a")), &field_terms.next()?);
    let mut postings = field_terms.postings(None, POSTINGS_ENUM_ALL)?;
    assert_doc_freq(&mut *postings, 0, 3)?;
    assert_doc_freq(&mut *postings, 1, 2)?;
    assert_doc_freq(&mut *postings, 3, 1)?;
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    assert_equals(&Some(utf8("b")), &field_terms.next()?);
    let mut postings = field_terms.postings(Some(postings), POSTINGS_ENUM_ALL)?;
    assert_doc_freq(&mut *postings, 2, 1)?;
    assert_doc_freq(&mut *postings, 4, 3)?;
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    assert_null(&field_terms.next()?);
    reader.close()?;
    drop(w);
    dir.close()
}

/// `testSortDocsAndFreqsAndPositions` and, with `offsets`,
/// `testSortDocsAndFreqsAndPositionsAndOffsets`.
fn sort_docs_and_positions(offsets: bool) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let config = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::new(&*random()),
    )));
    config.set_index_sort(sort_of(vec![sort_field("sort", SortFieldType::Long)?])?)?;
    let w = IndexWriter::new(dir.as_directory(), config)?;
    let ft = FieldType::new();
    ft.set_index_options(if offsets {
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS
    } else {
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
    })?;
    ft.set_tokenized(true)?;
    ft.freeze();
    add_sort_docs(
        &w,
        &ft,
        &[
            (0, &["a a b"]),
            (1, &["b"]),
            (-1, &["b a b b"]),
            (2, &["a"]),
            (3, &["b b"]),
        ],
    )?;
    w.force_merge(1)?;
    let reader = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let leaf_reader = only_leaf(&reader)?;
    let mut field_terms = leaf_reader
        .terms("field")?
        .unwrap_or_else(|| npe("terms(\"field\")"))
        .iterator()?;
    let o = |start: i32, end: i32| if offsets { Some((start, end)) } else { None };
    assert_equals(&Some(utf8("a")), &field_terms.next()?);
    let mut postings = field_terms.postings(None, POSTINGS_ENUM_ALL)?;
    assert_doc_freq(&mut *postings, 0, 1)?;
    assert_position(&mut *postings, 1, o(2, 3))?;
    assert_doc_freq(&mut *postings, 1, 2)?;
    assert_position(&mut *postings, 0, o(0, 1))?;
    assert_position(&mut *postings, 1, o(2, 3))?;
    assert_doc_freq(&mut *postings, 3, 1)?;
    assert_position(&mut *postings, 0, o(0, 1))?;
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    assert_equals(&Some(utf8("b")), &field_terms.next()?);
    let mut postings = field_terms.postings(Some(postings), POSTINGS_ENUM_ALL)?;
    assert_doc_freq(&mut *postings, 0, 3)?;
    assert_position(&mut *postings, 0, o(0, 1))?;
    assert_position(&mut *postings, 2, o(4, 5))?;
    assert_position(&mut *postings, 3, o(6, 7))?;
    assert_doc_freq(&mut *postings, 1, 1)?;
    assert_position(&mut *postings, 2, o(4, 5))?;
    assert_doc_freq(&mut *postings, 2, 1)?;
    assert_position(&mut *postings, 0, o(0, 1))?;
    assert_doc_freq(&mut *postings, 4, 2)?;
    assert_position(&mut *postings, 0, o(0, 1))?;
    assert_position(&mut *postings, 1, o(2, 3))?;
    assert_equals(&NO_MORE_DOCS, &postings.next_doc()?);
    assert_null(&field_terms.next()?);
    reader.close()?;
    drop(w);
    dir.close()
}

/// Port of `TestIndexSorting.testSortDocsAndFreqsAndPositions`.
fn test_sort_docs_and_freqs_and_positions() -> Result<()> {
    sort_docs_and_positions(false)
}

/// Port of `TestIndexSorting.testSortDocsAndFreqsAndPositionsAndOffsets`.
fn test_sort_docs_and_freqs_and_positions_and_offsets() -> Result<()> {
    sort_docs_and_positions(true)
}

/// Port of `TestIndexSorting.testParentFieldNotConfigured`.
fn test_parent_field_not_configured() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Int)?])?)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    let message = iae_message_of(|| writer.add_documents(&[Document::new(), Document::new()]));
    assert_equals(
        &"a parent field must be set in order to use document blocks with index sorting; see IndexWriterConfig#setParentField".to_string(),
        &message,
    );
    writer.close()?;
    drop(writer);
    dir.close()
}

/// Port of `TestIndexSorting.testBlockContainsParentField`.
fn test_block_contains_parent_field() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let parent_field = "parent";
    iwc.set_parent_field(Some(parent_field.to_string()));
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Int)?])?)?;
    let writer = IndexWriter::new(dir.as_directory(), iwc)?;
    {
        let reserved = "\"parent\" is a reserved field and should not be added to any document";
        let first: Box<dyn Fn()> = Box::new(|| {
            let message = iae_message_of(|| {
                let mut doc = Document::new();
                doc.add(ndv("parent", 0));
                writer.add_documents(&[doc, Document::new()])
            });
            assert_equals(&reserved.to_string(), &message);
        });
        let second: Box<dyn Fn()> = Box::new(|| {
            let message = iae_message_of(|| {
                let mut doc = Document::new();
                doc.add(ndv("parent", 0));
                writer.add_documents(&[Document::new(), doc])
            });
            assert_equals(&reserved.to_string(), &message);
        });
        let mut runnabels = vec![first, second];
        shuffle(&mut runnabels, &*random());
        for runnable in &runnabels {
            runnable();
        }
    }
    writer.close()?;
    drop(writer);
    dir.close()
}

/// `new LogMergePolicy` as `newLogMergePolicy()`, with a merge factor of at
/// least 3.
fn merge_factor_above_two() -> Result<LogMergePolicy> {
    let policy = LuceneTestCase::new_log_merge_policy();
    // make sure that merge factor is always > 2
    if policy.get_merge_factor() <= 2 {
        policy.set_merge_factor(3)?;
    }
    Ok(policy)
}

/// Port of `TestIndexSorting.testIndexSortWithBlocks`.
fn test_index_sort_with_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let (codec, codec_state) = asserting_needs_index_sort_codec();
    iwc.set_codec(codec)?;
    let parent_field = "parent";
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Int)?])?)?;
    iwc.set_parent_field(Some(parent_field.to_string()));
    iwc.set_merge_policy(Some(Arc::new(merge_factor_above_two()?)))?;

    // add already sorted documents
    codec_state.num_calls.store(0, Ordering::SeqCst);
    codec_state.needs_index_sort.store(false, Ordering::SeqCst);
    {
        let w = IndexWriter::new(dir.as_directory(), iwc)?;
        let num_docs = random().next_int_range(50, 100);
        for i in 0..num_docs {
            let mut child1 = Document::new();
            child1.add(string_field("id", &i.to_string(), Store::YES)?);
            child1.add(ndv("id", i as i64));
            child1.add(ndv("child", 1));
            child1.add(ndv("foo", random().next_int() as i64));
            let mut child2 = Document::new();
            child2.add(string_field("id", &i.to_string(), Store::YES)?);
            child2.add(ndv("id", i as i64));
            child2.add(ndv("child", 2));
            child2.add(ndv("foo", random().next_int() as i64));
            let mut parent = Document::new();
            parent.add(string_field("id", &i.to_string(), Store::YES)?);
            parent.add(ndv("id", i as i64));
            parent.add(ndv("foo", random().next_int() as i64));
            w.add_documents(&[child1, child2, parent])?;
            if LuceneTestCase::rarely() {
                w.commit()?;
            }
        }
        w.commit()?;
        if random().next_boolean() {
            w.force_merge_with(1, true)?;
        }
        w.close()?;
    }

    {
        let reader = ir(&directory_reader::open(dir.as_directory())?);
        for ctx in Arc::clone(&reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let mut parent_disi = numeric(&leaf, parent_field)?;
            let mut ids = numeric(&leaf, "id")?;
            let mut children = numeric(&leaf, "child")?;
            let mut expected_doc_id = 2;
            loop {
                let doc = parent_disi.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                assert_equals(&-1, &parent_disi.long_value()?);
                assert_equals(&expected_doc_id, &doc);
                let id = ids.next_doc()?;
                let child1_id = ids.long_value()?;
                assert_equals(&id, &children.next_doc()?);
                let child1 = children.long_value()?;
                assert_equals(&1, &child1);

                let id = ids.next_doc()?;
                let child2_id = ids.long_value()?;
                assert_equals(&id, &children.next_doc()?);
                let child2 = children.long_value()?;
                assert_equals(&2, &child2);

                let id_parent = ids.next_doc()?;
                assert_equals(&(id + 1), &id_parent);
                let parent = ids.long_value()?;
                assert_equals(&child1_id, &parent);
                assert_equals(&child2_id, &parent);
                expected_doc_id += 3;
            }
        }
        reader.close()?;
    }
    drop(codec_state);
    dir.close()
}

/// A document of `testMixRandomDocumentsWithBlocks`: `id`, `type`, an
/// optional `child_ord` or `num_children`, and a random `foo`.
fn block_doc(i: i32, doc_type: i64, extra: Option<(&str, i64)>) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(string_field("id", &i.to_string(), Store::YES)?);
    doc.add(ndv("type", doc_type));
    if let Some((name, value)) = extra {
        doc.add(ndv(name, value));
    }
    doc.add(ndv("foo", random().next_int() as i64));
    Ok(doc)
}

/// Port of `TestIndexSorting.testMixRandomDocumentsWithBlocks`.
fn test_mix_random_documents_with_blocks() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    let (codec, _codec_state) = asserting_needs_index_sort_codec();
    iwc.set_codec(codec)?;
    let parent_field = "parent";
    iwc.set_index_sort(sort_of(vec![sort_field("foo", SortFieldType::Int)?])?)?;
    iwc.set_parent_field(Some(parent_field.to_string()));
    let random_index_writer =
        RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let num_docs = random().next_int_range(100, 1000);
    for i in 0..num_docs {
        if LuceneTestCase::rarely() {
            random_index_writer
                .delete_documents(term("id", &random().next_int_range(0, i + 1).to_string()))?;
        }
        let mut docs = Vec::new();
        // The Java `switch` falls through from `case 4` down to `case 1`.
        let case = random().next_int_bound(100) % 5;
        if case >= 4 {
            docs.push(block_doc(i, 2, Some(("child_ord", 3)))?);
        }
        if case >= 3 {
            docs.push(block_doc(i, 2, Some(("child_ord", 2)))?);
        }
        if case >= 2 {
            docs.push(block_doc(i, 2, Some(("child_ord", 1)))?);
        }
        if case >= 1 {
            let num_children = docs.len() as i64;
            docs.push(block_doc(i, 1, Some(("num_children", num_children)))?);
            random_index_writer.add_documents(&docs)?;
        } else {
            let single = block_doc(i, 0, None)?;
            random_index_writer.add_document(&single)?;
        }
        if LuceneTestCase::rarely() {
            random_index_writer.force_merge(1)?;
        }
        random_index_writer.commit()?;
    }

    random_index_writer.close()?;
    drop(random_index_writer);
    {
        let reader = ir(&directory_reader::open(dir.as_directory())?);
        for ctx in Arc::clone(&reader).leaves()? {
            let leaf = ctx.leaf_reader();
            let parent_disi = leaf.get_numeric_doc_values(parent_field)?;
            assert_not_null(&parent_disi);
            let mut parent_disi = parent_disi.unwrap_or_else(|| npe("parentDISI"));
            let mut type_values = numeric(&leaf, "type")?;
            let mut child_ord = leaf.get_numeric_doc_values("child_ord")?;
            let mut num_children = leaf.get_numeric_doc_values("num_children")?;
            let mut num_current_children = 0;
            let mut total_pending_children = 0;
            let mut child_id: Option<String> = None;
            for i in 0..leaf.max_doc() {
                let live = leaf.get_live_docs().is_none_or(|l| l.get(i as usize));
                if !live {
                    continue;
                }
                Assert::assert_true(type_values.advance_exact(i)?);
                let type_value = type_values.long_value()? as i32;
                match type_value {
                    2 => {
                        Assert::assert_false(parent_disi.advance_exact(i)?);
                        let child_ord = child_ord.as_mut().unwrap_or_else(|| npe("childOrd"));
                        Assert::assert_true(child_ord.advance_exact(i)?);
                        if num_current_children == 0 {
                            // first child
                            child_id = leaf.stored_fields()?.document(i)?.get("id");
                            total_pending_children = child_ord.long_value()? as i32 - 1;
                        } else {
                            assert_not_null(&child_id);
                            assert_equals(
                                &(total_pending_children as i64),
                                &child_ord.long_value()?,
                            );
                            total_pending_children -= 1;
                            assert_equals(&child_id, &leaf.stored_fields()?.document(i)?.get("id"));
                        }
                        num_current_children += 1;
                    }
                    1 => {
                        Assert::assert_true(parent_disi.advance_exact(i)?);
                        assert_equals(&-1, &parent_disi.long_value()?);
                        if let Some(child_ord) = child_ord.as_mut() {
                            Assert::assert_false(child_ord.advance_exact(i)?);
                        }
                        let num_children =
                            num_children.as_mut().unwrap_or_else(|| npe("numChildren"));
                        Assert::assert_true(num_children.advance_exact(i)?);
                        assert_equals(&0, &total_pending_children);
                        assert_equals(&(num_current_children as i64), &num_children.long_value()?);
                        if num_current_children > 0 {
                            assert_equals(&child_id, &leaf.stored_fields()?.document(i)?.get("id"));
                        } else {
                            assert_null(&child_id);
                        }
                        num_current_children = 0;
                        child_id = None;
                    }
                    0 => {
                        assert_equals(&-1, &parent_disi.long_value()?);
                        Assert::assert_true(parent_disi.advance_exact(i)?);
                        if let Some(child_ord) = child_ord.as_mut() {
                            Assert::assert_false(child_ord.advance_exact(i)?);
                        }
                        if let Some(num_children) = num_children.as_mut() {
                            Assert::assert_false(num_children.advance_exact(i)?);
                        }
                    }
                    _ => Assert::fail(""),
                }
            }
        }
        reader.close()?;
    }
    dir.close()
}
