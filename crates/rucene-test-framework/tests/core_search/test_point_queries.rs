//! Port of `lucene/core/src/test/org/apache/lucene/search/TestPointQueries.java`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::{Assert, BeforeClass};
use rucene::codecs::lucene90::points::{
    Lucene90PointsReader, Lucene90PointsWriter, VERSION_CURRENT,
};
use rucene::codecs::points::{PointsFormat, PointsReader, PointsWriter};
use rucene::codecs::state::{SegmentReadState, SegmentWriteState};
use rucene::codecs::{Codec, FilterCodec};
use rucene::document::{
    BinaryPoint, Document, DoublePoint, FloatPoint, IntPoint, LongPoint, NumericDocValuesField,
    SortedNumericDocValuesField, Store, StringField,
};
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader::{self, DirectoryReader};
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::multi_doc_values::MultiDocValues;
use rucene::index::point_values::{MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES};
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::point_range_query::DimensionFormatter;
use rucene::search::{
    doc_id_set_iterator, query_to_string, DocIdSetIterator, FieldExistsQuery, MatchAllDocsQuery,
    MatchNoDocsQuery, PointFormatter, PointInSetQuery, PointRangeQuery, Query, ScoreMode,
};
use rucene::util::bkd::BKDConfig;
use rucene::util::{BytesRef, BytesRefIterator, NumericUtils};
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::FixedBitSetCollector;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::{Nightly, SuppressCodecs};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::static_hook;
use crate::core_support::{
    arrays_to_string, assert_array_equals, assert_equals, assert_equals_msg, class_annotated,
    expect_throws_panic, random, test, verbose, JavaHashSet, IAE,
};

/// `TestPointQueries`.
pub fn class_def() -> Arc<Class> {
    class_annotated(
        "org.apache.lucene.search.TestPointQueries",
        vec![
            static_hook("beforeClass", BeforeClass, before_class),
            test("testBasicInts", test_basic_ints),
            test("testBasicFloats", test_basic_floats),
            test("testBasicLongs", test_basic_longs),
            test("testBasicDoubles", test_basic_doubles),
            test("testCrazyDoubles", test_crazy_doubles),
            test("testCrazyFloats", test_crazy_floats),
            test("testAllEqual", test_all_equal),
            test("testRandomLongsTiny", test_random_longs_tiny),
            test("testRandomLongsMedium", test_random_longs_medium),
            test("testRandomLongsBig", test_random_longs_big),
            test("testLongEncode", test_long_encode),
            test("testRandomBinaryTiny", test_random_binary_tiny),
            test("testRandomBinaryMedium", test_random_binary_medium),
            test("testMinMaxLong", test_min_max_long),
            test("testBasicSortedSet", test_basic_sorted_set),
            test("testLongMinMaxNumeric", test_long_min_max_numeric),
            test("testLongMinMaxSortedSet", test_long_min_max_sorted_set),
            test("testSortedSetNoOrdsMatch", test_sorted_set_no_ords_match),
            test("testNumericNoValuesMatch", test_numeric_no_values_match),
            test("testNoDocs", test_no_docs),
            test("testWrongNumDims", test_wrong_num_dims),
            test("testWrongNumBytes", test_wrong_num_bytes),
            test(
                "testAllPointDocsWereDeletedAndThenMergedAgain",
                test_all_point_docs_were_deleted_and_then_merged_again,
            ),
            test("testExactPoints", test_exact_points),
            test("testToString", test_to_string),
            test("testRandomPointInSetQuery", test_random_point_in_set_query),
            test(
                "testBasicMultiDimPointInSetQuery",
                test_basic_multi_dim_point_in_set_query,
            ),
            test(
                "testBasicMultiValueMultiDimPointInSetQuery",
                test_basic_multi_value_multi_dim_point_in_set_query,
            ),
            test(
                "testManyEqualValuesMultiDimPointInSetQuery",
                test_many_equal_values_multi_dim_point_in_set_query,
            ),
            test(
                "testInvalidMultiDimPointInSetQuery",
                test_invalid_multi_dim_point_in_set_query,
            ),
            test("testBasicPointInSetQuery", test_basic_point_in_set_query),
            test("testPointIntSetBoxed", test_point_int_set_boxed),
            test(
                "testBasicMultiValuedPointInSetQuery",
                test_basic_multi_valued_point_in_set_query,
            ),
            test("testEmptyPointInSetQuery", test_empty_point_in_set_query),
            test(
                "testPointInSetQueryManyEqualValues",
                test_point_in_set_query_many_equal_values,
            ),
            test(
                "testPointRangeQueryManyEqualValues",
                test_point_range_query_many_equal_values,
            ),
            test(
                "testPointInSetQueryManyEqualValuesWithBigGap",
                test_point_in_set_query_many_equal_values_with_big_gap,
            ),
            test(
                "testInvalidPointInSetQuery",
                test_invalid_point_in_set_query,
            ),
            test(
                "testInvalidPointInSetBinaryQuery",
                test_invalid_point_in_set_binary_query,
            ),
            test(
                "testPointInSetQueryToString",
                test_point_in_set_query_to_string,
            ),
            test(
                "testPointInSetQueryGetPackedPoints",
                test_point_in_set_query_get_packed_points,
            ),
            test(
                "testRangeOptimizesIfAllPointsMatch",
                test_range_optimizes_if_all_points_match,
            ),
            test("testPointRangeWeightCount", test_point_range_weight_count),
            test("testPointRangeEquals", test_point_range_equals),
            test("testPointExactEquals", test_point_exact_equals),
            test("testPointInSetEquals", test_point_in_set_equals),
            test("testInvalidPointLength", test_invalid_point_length),
            test("testNextUp", test_next_up),
            test("testNextDown", test_next_down),
            test("testInversePointRange", test_inverse_point_range).annotate(Nightly),
            test(
                "testRangeQueryOptimizesRewrites",
                test_range_query_optimizes_rewrites,
            ),
            test(
                "testRangeQuerySkipsNonMatchingSegments",
                test_range_query_skips_non_matching_segments,
            ),
            test(
                "testPointInSetQuerySkipsNonMatchingSegments",
                test_point_in_set_query_skips_non_matching_segments,
            ),
            test(
                "testOutOfOrderValuesInPointInSetQuery",
                test_out_of_order_values_in_point_in_set_query,
            ),
        ],
        |b| {
            b.annotate(SuppressCodecs {
                value: vec!["SimpleText".to_string()],
            })
        },
    )
}

// Controls what range of values we randomly generate, so we sometimes test narrow ranges:
/// `static long valueMid`.
static VALUE_MID: AtomicI64 = AtomicI64::new(0);
/// `static int valueRange`.
static VALUE_RANGE: AtomicI32 = AtomicI32::new(0);

/// `Double.MIN_VALUE`, the smallest positive subnormal `double`.
const DOUBLE_MIN_VALUE: f64 = f64::from_bits(1);
/// `Float.MIN_VALUE`, the smallest positive subnormal `float`.
const FLOAT_MIN_VALUE: f32 = f32::from_bits(1);

fn before_class() -> Result<()> {
    if random().next_boolean() {
        VALUE_MID.store(random().next_long(), Ordering::Relaxed);
        if random().next_boolean() {
            // Wide range
            VALUE_RANGE.store(
                TestUtil::next_int(&*random(), 1, i32::MAX),
                Ordering::Relaxed,
            );
        } else {
            // Narrow range
            VALUE_RANGE.store(TestUtil::next_int(&*random(), 1, 100000), Ordering::Relaxed);
        }
        if verbose() {
            println!(
                "TEST: will generate long values {} +/- {}",
                VALUE_MID.load(Ordering::Relaxed),
                VALUE_RANGE.load(Ordering::Relaxed)
            );
        }
    } else {
        // All longs
        VALUE_RANGE.store(0, Ordering::Relaxed);
        if verbose() {
            println!("TEST: will generate all long values");
        }
    }
    Ok(())
}

/// `new IndexWriter(dir, new IndexWriterConfig(new MockAnalyzer(random())))`.
fn mock_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        dir.as_directory(),
        IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random())))),
    )
}

/// `new IndexSearcher(r)`.
fn plain_searcher(r: &Arc<dyn DirectoryReader>) -> Result<IndexSearcher> {
    IndexSearcher::new(Arc::clone(r) as Arc<dyn IndexReader>)
}

/// A document holding the single field `field`.
fn doc_with(field: Box<dyn rucene::index::IndexableField>) -> Document {
    let mut doc = Document::new();
    doc.add(field);
    doc
}

/// Port of `TestPointQueries.testBasicInts`.
fn test_basic_ints() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = mock_writer(&dir)?;

    w.add_document(&doc_with(Box::new(IntPoint::new("point", &[-7])?)))?;
    w.add_document(&doc_with(Box::new(IntPoint::new("point", &[0])?)))?;
    w.add_document(&doc_with(Box::new(IntPoint::new("point", &[3])?)))?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = plain_searcher(&r)?;
    assert_equals(
        &2,
        &s.count(IntPoint::new_range_query("point", &[-8], &[1])?)?,
    );
    assert_equals(
        &3,
        &s.count(IntPoint::new_range_query("point", &[-7], &[3])?)?,
    );
    assert_equals(&1, &s.count(IntPoint::new_exact_query("point", -7)?)?);
    assert_equals(&0, &s.count(IntPoint::new_exact_query("point", -6)?)?);
    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testBasicFloats`.
fn test_basic_floats() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = mock_writer(&dir)?;

    w.add_document(&doc_with(Box::new(FloatPoint::new("point", &[-7.0])?)))?;
    w.add_document(&doc_with(Box::new(FloatPoint::new("point", &[0.0])?)))?;
    w.add_document(&doc_with(Box::new(FloatPoint::new("point", &[3.0])?)))?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = plain_searcher(&r)?;
    assert_equals(
        &2,
        &s.count(FloatPoint::new_range_query("point", &[-8.0], &[1.0])?)?,
    );
    assert_equals(
        &3,
        &s.count(FloatPoint::new_range_query("point", &[-7.0], &[3.0])?)?,
    );
    assert_equals(&1, &s.count(FloatPoint::new_exact_query("point", -7.0)?)?);
    assert_equals(&0, &s.count(FloatPoint::new_exact_query("point", -6.0)?)?);
    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testBasicLongs`.
fn test_basic_longs() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = mock_writer(&dir)?;

    w.add_document(&doc_with(Box::new(LongPoint::new("point", &[-7])?)))?;
    w.add_document(&doc_with(Box::new(LongPoint::new("point", &[0])?)))?;
    w.add_document(&doc_with(Box::new(LongPoint::new("point", &[3])?)))?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = plain_searcher(&r)?;
    assert_equals(
        &2,
        &s.count(LongPoint::new_range_query("point", &[-8], &[1])?)?,
    );
    assert_equals(
        &3,
        &s.count(LongPoint::new_range_query("point", &[-7], &[3])?)?,
    );
    assert_equals(&1, &s.count(LongPoint::new_exact_query("point", -7)?)?);
    assert_equals(&0, &s.count(LongPoint::new_exact_query("point", -6)?)?);
    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testBasicDoubles`.
fn test_basic_doubles() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = mock_writer(&dir)?;

    w.add_document(&doc_with(Box::new(DoublePoint::new("point", &[-7.0])?)))?;
    w.add_document(&doc_with(Box::new(DoublePoint::new("point", &[0.0])?)))?;
    w.add_document(&doc_with(Box::new(DoublePoint::new("point", &[3.0])?)))?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = plain_searcher(&r)?;
    assert_equals(
        &2,
        &s.count(DoublePoint::new_range_query("point", &[-8.0], &[1.0])?)?,
    );
    assert_equals(
        &3,
        &s.count(DoublePoint::new_range_query("point", &[-7.0], &[3.0])?)?,
    );
    assert_equals(&1, &s.count(DoublePoint::new_exact_query("point", -7.0)?)?);
    assert_equals(&0, &s.count(DoublePoint::new_exact_query("point", -6.0)?)?);
    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testCrazyDoubles`.
fn test_crazy_doubles() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = mock_writer(&dir)?;

    for v in [
        f64::NEG_INFINITY,
        -0.0,
        0.0,
        DOUBLE_MIN_VALUE,
        f64::MAX,
        f64::INFINITY,
        f64::NAN,
    ] {
        w.add_document(&doc_with(Box::new(DoublePoint::new("point", &[v])?)))?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = plain_searcher(&r)?;

    // exact queries
    assert_equals(
        &1,
        &s.count(DoublePoint::new_exact_query("point", f64::NEG_INFINITY)?)?,
    );
    assert_equals(&1, &s.count(DoublePoint::new_exact_query("point", -0.0)?)?);
    assert_equals(&1, &s.count(DoublePoint::new_exact_query("point", 0.0)?)?);
    assert_equals(
        &1,
        &s.count(DoublePoint::new_exact_query("point", DOUBLE_MIN_VALUE)?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_exact_query("point", f64::MAX)?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_exact_query("point", f64::INFINITY)?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_exact_query("point", f64::NAN)?)?,
    );

    // set query
    let set = [
        f64::MAX,
        f64::NAN,
        0.0,
        f64::NEG_INFINITY,
        DOUBLE_MIN_VALUE,
        -0.0,
        f64::INFINITY,
    ];
    assert_equals(&7, &s.count(DoublePoint::new_set_query("point", &set)?)?);

    // ranges
    let range = |a: f64, b: f64| DoublePoint::new_range_query("point", &[a], &[b]);
    assert_equals(&2, &s.count(range(f64::NEG_INFINITY, -0.0)?)?);
    assert_equals(&2, &s.count(range(-0.0, 0.0)?)?);
    assert_equals(&2, &s.count(range(0.0, DOUBLE_MIN_VALUE)?)?);
    assert_equals(&2, &s.count(range(DOUBLE_MIN_VALUE, f64::MAX)?)?);
    assert_equals(&2, &s.count(range(f64::MAX, f64::INFINITY)?)?);
    assert_equals(&2, &s.count(range(f64::INFINITY, f64::NAN)?)?);

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testCrazyFloats`.
fn test_crazy_floats() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = mock_writer(&dir)?;

    for v in [
        f32::NEG_INFINITY,
        -0.0,
        0.0,
        FLOAT_MIN_VALUE,
        f32::MAX,
        f32::INFINITY,
        f32::NAN,
    ] {
        w.add_document(&doc_with(Box::new(FloatPoint::new("point", &[v])?)))?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = plain_searcher(&r)?;

    // exact queries
    assert_equals(
        &1,
        &s.count(FloatPoint::new_exact_query("point", f32::NEG_INFINITY)?)?,
    );
    assert_equals(&1, &s.count(FloatPoint::new_exact_query("point", -0.0)?)?);
    assert_equals(&1, &s.count(FloatPoint::new_exact_query("point", 0.0)?)?);
    assert_equals(
        &1,
        &s.count(FloatPoint::new_exact_query("point", FLOAT_MIN_VALUE)?)?,
    );
    assert_equals(
        &1,
        &s.count(FloatPoint::new_exact_query("point", f32::MAX)?)?,
    );
    assert_equals(
        &1,
        &s.count(FloatPoint::new_exact_query("point", f32::INFINITY)?)?,
    );
    assert_equals(
        &1,
        &s.count(FloatPoint::new_exact_query("point", f32::NAN)?)?,
    );

    // set query
    let set = [
        f32::MAX,
        f32::NAN,
        0.0,
        f32::NEG_INFINITY,
        FLOAT_MIN_VALUE,
        -0.0,
        f32::INFINITY,
    ];
    assert_equals(&7, &s.count(FloatPoint::new_set_query("point", &set)?)?);

    // ranges
    let range = |a: f32, b: f32| FloatPoint::new_range_query("point", &[a], &[b]);
    assert_equals(&2, &s.count(range(f32::NEG_INFINITY, -0.0)?)?);
    assert_equals(&2, &s.count(range(-0.0, 0.0)?)?);
    assert_equals(&2, &s.count(range(0.0, FLOAT_MIN_VALUE)?)?);
    assert_equals(&2, &s.count(range(FLOAT_MIN_VALUE, f32::MAX)?)?);
    assert_equals(&2, &s.count(range(f32::MAX, f32::INFINITY)?)?);
    assert_equals(&2, &s.count(range(f32::INFINITY, f32::NAN)?)?);

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testAllEqual`.
fn test_all_equal() -> Result<()> {
    let num_values = LuceneTestCase::at_least(1000);
    let value = random_value();
    if verbose() {
        println!("TEST: use same value={value}");
    }
    let values = vec![value; num_values as usize];

    verify_longs(Arc::new(values), None)
}

/// Port of `TestPointQueries.testRandomLongsTiny`.
fn test_random_longs_tiny() -> Result<()> {
    // Make sure single-leaf-node case is OK:
    do_test_random_longs(10)
}

/// Port of `TestPointQueries.testRandomLongsMedium`.
fn test_random_longs_medium() -> Result<()> {
    do_test_random_longs(1000)
}

/// Port of `TestPointQueries.testRandomLongsBig`.
fn test_random_longs_big() -> Result<()> {
    do_test_random_longs(20_000)
}

/// `private void doTestRandomLongs(int count)`.
fn do_test_random_longs(count: i32) -> Result<()> {
    let num_values = TestUtil::next_int(&*random(), count, count * 2);

    if verbose() {
        println!("TEST: numValues={num_values}");
    }

    let mut values = vec![0i64; num_values as usize];
    let mut ids = vec![0i32; num_values as usize];

    let single_valued = random().next_boolean();

    let same_value_pct = random().next_int_bound(100);

    let mut id = 0;
    for ord in 0..num_values as usize {
        if ord > 0 && random().next_int_bound(100) < same_value_pct {
            // Identical to old value
            values[ord] = values[random().next_int_bound(ord as i32) as usize];
        } else {
            values[ord] = random_value();
        }

        ids[ord] = id;
        if single_valued || random().next_int_bound(2) == 1 {
            id += 1;
        }
    }

    verify_longs(Arc::new(values), Some(ids))
}

/// Port of `TestPointQueries.testLongEncode`.
fn test_long_encode() -> Result<()> {
    for _i in 0..10000 {
        let v = random().next_long();
        let mut tmp = [0u8; 8];
        NumericUtils::long_to_sortable_bytes(v, &mut tmp, 0);
        let v2 = NumericUtils::sortable_bytes_to_long(&tmp, 0);
        assert_equals_msg(
            Some(&format!("got bytes={}", java_bytes_to_string(&tmp))),
            &v,
            &v2,
        );
    }
    Ok(())
}

/// `Arrays.toString(byte[])`: the signed values of the bytes.
fn java_bytes_to_string(bytes: &[u8]) -> String {
    let signed: Vec<i8> = bytes.iter().map(|&b| b as i8).collect();
    arrays_to_string(&signed)
}

/// `Thread.currentThread().getName()`.
fn current_thread_name() -> String {
    Thread::current().name()
}

/// verify for long values
///
/// `private static void verifyLongs(long[] values, int[] ids)`.
fn verify_longs(values: Arc<Vec<i64>>, ids: Option<Vec<i32>>) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();

    // Else we can get O(N^2) merging:
    let mbd = iwc.max_buffered_docs();
    if mbd != -1 && mbd < (values.len() / 100) as i32 {
        iwc.set_max_buffered_docs((values.len() / 100) as i32)?;
    }
    iwc.set_codec(get_codec())?;
    let mut dir = if values.len() > 100000 {
        LuceneTestCase::new_maybe_virus_checking_fs_directory(
            &LuceneTestCase::create_temp_dir_prefix("TestRangeTree"),
        )
    } else {
        LuceneTestCase::new_maybe_virus_checking_directory()
    };

    /*
    The point range query chooses only considers using an inverse BKD visitor if
    there is exactly one value per document. If any document misses a value that
    code is not exercised. Using a nextBoolean() here increases the likelihood
    that there is no missing values, making the test more likely to test that code.
    */
    let missing_pct = if random().next_boolean() {
        0
    } else {
        random().next_int_bound(100)
    };
    let deleted_pct = random().next_int_bound(100);
    if verbose() {
        println!("  missingPct={missing_pct}");
        println!("  deletedPct={deleted_pct}");
    }

    let mut missing: HashSet<i32> = HashSet::new();
    let mut deleted: HashSet<i32> = HashSet::new();

    let mut doc: Option<Document> = None;
    let mut last_id = -1;

    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    for ord in 0..values.len() {
        let id = match &ids {
            None => ord as i32,
            Some(ids) => ids[ord],
        };
        if id != last_id {
            if random().next_int_bound(100) < missing_pct {
                missing.insert(id);
                if verbose() {
                    println!("  missing id={id}");
                }
            }

            if let Some(d) = doc.take() {
                w.add_document(&d)?;
                if random().next_int_bound(100) < deleted_pct {
                    let id_to_delete = random().next_int_bound(id);
                    w.delete_documents_by_terms(vec![Term::from_text(
                        "id",
                        &id_to_delete.to_string(),
                    )])?;
                    deleted.insert(id_to_delete);
                    if verbose() {
                        println!("  delete id={id_to_delete}");
                    }
                }
            }

            let mut d = Document::new();
            d.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                &id.to_string(),
                Store::NO,
            )));
            d.add(Box::new(NumericDocValuesField::new("id", i64::from(id))));
            doc = Some(d);
            last_id = id;
        }

        if !missing.contains(&id) {
            let d = doc.as_mut().expect("doc was created for this id");
            d.add(Box::new(LongPoint::new(
                "sn_value",
                &[values[id as usize]],
            )?));
            let mut bytes = vec![0u8; 8];
            NumericUtils::long_to_sortable_bytes(values[id as usize], &mut bytes, 0);
            d.add(Box::new(BinaryPoint::new(
                "ss_value",
                &[BytesRef::new(bytes)],
            )?));
        }
    }

    w.add_document(doc.as_ref().expect("at least one value"))?;

    if random().next_boolean() {
        if verbose() {
            println!("  forceMerge(1)");
        }
        w.force_merge(1)?;
    }
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    let s = Arc::new(LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false));

    let num_threads = TestUtil::next_int(&*random(), 2, 5);

    if verbose() {
        println!("TEST: use {num_threads} query threads; searcher={s:?}");
    }

    let mut threads = Vec::new();
    let iters = LuceneTestCase::at_least(100);

    let starting_gun = Arc::new(crate::core_search::support::CountDownLatch::new(1));
    let failed = Arc::new(AtomicBool::new(false));
    let missing = Arc::new(missing);
    let deleted = Arc::new(deleted);
    let r_index: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;

    for i in 0..num_threads {
        let starting_gun = Arc::clone(&starting_gun);
        let failed = Arc::clone(&failed);
        let s = Arc::clone(&s);
        let r = Arc::clone(&r_index);
        let values = Arc::clone(&values);
        let missing = Arc::clone(&missing);
        let deleted = Arc::clone(&deleted);
        let thread = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();

                let mut iter = 0;
                while iter < iters && !failed.load(Ordering::SeqCst) {
                    let mut lower = random_value();
                    let mut upper = random_value();

                    if upper < lower {
                        std::mem::swap(&mut lower, &mut upper);
                    }

                    if verbose() {
                        println!(
                            "\n{}: TEST: iter={iter} value={lower} TO {upper}",
                            current_thread_name()
                        );
                        let mut tmp = [0u8; 8];
                        NumericUtils::long_to_sortable_bytes(lower, &mut tmp, 0);
                        println!("  lower bytes={}", java_bytes_to_string(&tmp));
                        NumericUtils::long_to_sortable_bytes(upper, &mut tmp, 0);
                        println!("  upper bytes={}", java_bytes_to_string(&tmp));
                    }

                    let query: Arc<dyn Query> = if random().next_boolean() {
                        LongPoint::new_range_query("sn_value", &[lower], &[upper])?
                    } else {
                        let mut lower_bytes = vec![0u8; 8];
                        NumericUtils::long_to_sortable_bytes(lower, &mut lower_bytes, 0);
                        let mut upper_bytes = vec![0u8; 8];
                        NumericUtils::long_to_sortable_bytes(upper, &mut upper_bytes, 0);
                        BinaryPoint::new_range_query(
                            "ss_value",
                            &[BytesRef::new(lower_bytes)],
                            &[BytesRef::new(upper_bytes)],
                        )?
                    };

                    if verbose() {
                        println!(
                            "{}:  using query: {}",
                            current_thread_name(),
                            query_to_string(&*query)
                        );
                    }

                    let hits = s.search_with_manager(
                        Arc::clone(&query),
                        &FixedBitSetCollector::create_manager(r.max_doc()),
                    )?;

                    if verbose() {
                        println!(
                            "{}:  hitCount: {}",
                            current_thread_name(),
                            hits.cardinality()
                        );
                    }

                    let mut doc_id_to_id = MultiDocValues::get_numeric_values(&r, "id")?
                        .unwrap_or_else(|| {
                            randomizedtesting::jdk::exception::throw_without_message(
                                "java.lang.NullPointerException",
                            )
                        });

                    for doc_id in 0..r.max_doc() {
                        assert_equals(&doc_id, &doc_id_to_id.next_doc()?);
                        let id = doc_id_to_id.long_value()? as i32;
                        let expected = !missing.contains(&id)
                            && !deleted.contains(&id)
                            && values[id as usize] >= lower
                            && values[id as usize] <= upper;
                        if hits.get(doc_id as usize) != expected {
                            // We do exact quantized comparison so the bbox query should never disagree:
                            Assert::fail(&format!(
                                "{}: iter={iter} id={id} docID={doc_id} value={} (range: {lower} TO {upper}) expected {expected} but got: {} deleted?={} query={}",
                                current_thread_name(),
                                values[id as usize],
                                hits.get(doc_id as usize),
                                deleted.contains(&id),
                                query_to_string(&*query)
                            ));
                        }
                    }
                    iter += 1;
                }
                Ok(())
            };
            if let Err(e) = run() {
                failed.store(true, Ordering::SeqCst);
                rethrow_java(e)
            }
        });
        thread.set_name(format!("T{i}"));
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    for thread in &threads {
        let _ = thread.join();
    }
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testRandomBinaryTiny`.
fn test_random_binary_tiny() -> Result<()> {
    do_test_random_binary(10)
}

/// Port of `TestPointQueries.testRandomBinaryMedium`.
fn test_random_binary_medium() -> Result<()> {
    do_test_random_binary(1000)
}

/// One document value: `byte[numDims][numBytesPerDim]`, shared between the
/// ords Java aliases to the same array.
type DocValue = Arc<Vec<Vec<u8>>>;

/// `private void doTestRandomBinary(int count)`.
fn do_test_random_binary(count: i32) -> Result<()> {
    let num_values = TestUtil::next_int(&*random(), count, count * 2);
    let num_bytes_per_dim = TestUtil::next_int(&*random(), 2, MAX_NUM_BYTES);
    let num_dims = TestUtil::next_int(&*random(), 1, MAX_INDEX_DIMENSIONS);

    let same_value_pct = random().next_int_bound(100);
    if verbose() {
        println!("TEST: sameValuePct={same_value_pct}");
    }

    let mut doc_values: Vec<DocValue> = Vec::with_capacity(num_values as usize);

    let single_valued = random().next_boolean();
    let mut ids = vec![0i32; num_values as usize];

    let mut id = 0;
    if verbose() {
        println!("Picking values: {num_values}");
    }
    for ord in 0..num_values as usize {
        if ord > 0 && random().next_int_bound(100) < same_value_pct {
            // Identical to old value
            let old = Arc::clone(&doc_values[random().next_int_bound(ord as i32) as usize]);
            doc_values.push(old);
        } else {
            // Make a new random value
            let mut values = Vec::with_capacity(num_dims as usize);
            for _dim in 0..num_dims {
                let mut v = vec![0u8; num_bytes_per_dim as usize];
                random().next_bytes(&mut v);
                values.push(v);
            }
            doc_values.push(Arc::new(values));
        }
        ids[ord] = id;
        if single_valued || random().next_int_bound(2) == 1 {
            id += 1;
        }
    }

    verify_binary(Arc::new(doc_values), Arc::new(ids), num_bytes_per_dim)
}

/// verify for byte[][] values
///
/// `private void verifyBinary(byte[][][] docValues, int[] ids, int numBytesPerDim)`.
fn verify_binary(
    doc_values: Arc<Vec<DocValue>>,
    ids: Arc<Vec<i32>>,
    num_bytes_per_dim: i32,
) -> Result<()> {
    let iwc = LuceneTestCase::new_index_writer_config();

    let num_dims = doc_values[0].len();
    let bytes_per_dim = doc_values[0][0].len();

    // Else we can get O(N^2) merging:
    let mbd = iwc.max_buffered_docs();
    if mbd != -1 && mbd < (doc_values.len() / 100) as i32 {
        iwc.set_max_buffered_docs((doc_values.len() / 100) as i32)?;
    }
    iwc.set_codec(get_codec())?;

    let mut dir = if doc_values.len() > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "TestPointQueries",
        ))
    } else {
        LuceneTestCase::new_directory()
    };

    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let num_values = doc_values.len();
    if verbose() {
        println!(
            "TEST: numValues={num_values} numDims={num_dims} numBytesPerDim={num_bytes_per_dim}"
        );
    }

    let missing_pct = random().next_int_bound(100);
    let deleted_pct = random().next_int_bound(100);
    if verbose() {
        println!("  missingPct={missing_pct}");
        println!("  deletedPct={deleted_pct}");
    }

    let mut missing: HashSet<i32> = HashSet::new();
    let mut deleted: HashSet<i32> = HashSet::new();

    let mut doc: Option<Document> = None;
    let mut last_id = -1;

    for ord in 0..num_values {
        if ord % 1000 == 0 && verbose() {
            println!("Adding docs: {ord}");
        }
        let id = ids[ord];
        if id != last_id {
            if random().next_int_bound(100) < missing_pct {
                missing.insert(id);
                if verbose() {
                    println!("  missing id={id}");
                }
            }

            if let Some(d) = doc.take() {
                w.add_document(&d)?;
                if random().next_int_bound(100) < deleted_pct {
                    let id_to_delete = random().next_int_bound(id);
                    w.delete_documents_by_terms(vec![Term::from_text(
                        "id",
                        &id_to_delete.to_string(),
                    )])?;
                    deleted.insert(id_to_delete);
                    if verbose() {
                        println!("  delete id={id_to_delete}");
                    }
                }
            }

            let mut d = Document::new();
            d.add(Box::new(LuceneTestCase::new_string_field(
                "id",
                &id.to_string(),
                Store::NO,
            )));
            d.add(Box::new(NumericDocValuesField::new("id", i64::from(id))));
            doc = Some(d);
            last_id = id;
        }

        if !missing.contains(&id) {
            let d = doc.as_mut().expect("doc was created for this id");
            let point: Vec<BytesRef> = doc_values[ord]
                .iter()
                .map(|v| BytesRef::new(v.clone()))
                .collect();
            d.add(Box::new(BinaryPoint::new("value", &point)?));
            d.add(Box::new(SortedNumericDocValuesField::new("value", 1)));
            if verbose() {
                println!("id={id}");
                for dim in 0..num_dims {
                    println!(
                        "  dim={dim} value={}",
                        bytes_to_string(Some(&doc_values[ord][dim]))
                    );
                }
            }
        }
    }

    w.add_document(doc.as_ref().expect("at least one value"))?;

    if random().next_boolean() {
        if verbose() {
            println!("  forceMerge(1)");
        }
        w.force_merge(1)?;
    }
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    let s = Arc::new(LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false));

    let num_threads = TestUtil::next_int(&*random(), 2, 5);

    if verbose() {
        println!("TEST: use {num_threads} query threads; searcher={s:?}");
    }

    let mut threads = Vec::new();
    let iters = LuceneTestCase::at_least(100);

    let starting_gun = Arc::new(crate::core_search::support::CountDownLatch::new(1));
    let failed = Arc::new(AtomicBool::new(false));
    let missing = Arc::new(missing);
    let deleted = Arc::new(deleted);
    let r_index: Arc<dyn IndexReader> = Arc::clone(&r) as Arc<dyn IndexReader>;

    for i in 0..num_threads {
        let starting_gun = Arc::clone(&starting_gun);
        let failed = Arc::clone(&failed);
        let s = Arc::clone(&s);
        let r = Arc::clone(&r_index);
        let doc_values = Arc::clone(&doc_values);
        let ids = Arc::clone(&ids);
        let missing = Arc::clone(&missing);
        let deleted = Arc::clone(&deleted);
        let thread = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();

                let mut iter = 0;
                while iter < iters && !failed.load(Ordering::SeqCst) {
                    let mut lower: Vec<Vec<u8>> = Vec::with_capacity(num_dims);
                    let mut upper: Vec<Vec<u8>> = Vec::with_capacity(num_dims);
                    for _dim in 0..num_dims {
                        let mut l = vec![0u8; bytes_per_dim];
                        random().next_bytes(&mut l);

                        let mut u = vec![0u8; bytes_per_dim];
                        random().next_bytes(&mut u);

                        if l.as_slice() > u.as_slice() {
                            std::mem::swap(&mut l, &mut u);
                        }
                        lower.push(l);
                        upper.push(u);
                    }

                    if verbose() {
                        println!("\n{}: TEST: iter={iter}", current_thread_name());
                        for dim in 0..num_dims {
                            println!(
                                "  dim={dim} {} TO {}",
                                bytes_to_string(Some(&lower[dim])),
                                bytes_to_string(Some(&upper[dim]))
                            );
                        }
                    }

                    let lower_refs: Vec<BytesRef> =
                        lower.iter().map(|v| BytesRef::new(v.clone())).collect();
                    let upper_refs: Vec<BytesRef> =
                        upper.iter().map(|v| BytesRef::new(v.clone())).collect();
                    let query = BinaryPoint::new_range_query("value", &lower_refs, &upper_refs)?;

                    if verbose() {
                        println!(
                            "{}:  using query: {}",
                            current_thread_name(),
                            query_to_string(&*query)
                        );
                    }

                    let hits = s.search_with_manager(
                        Arc::clone(&query),
                        &FixedBitSetCollector::create_manager(r.max_doc()),
                    )?;

                    if verbose() {
                        println!(
                            "{}:  hitCount: {}",
                            current_thread_name(),
                            hits.cardinality()
                        );
                    }

                    let mut expected: HashSet<i32> = HashSet::new();
                    for ord in 0..num_values {
                        let id = ids[ord];
                        if !missing.contains(&id)
                            && !deleted.contains(&id)
                            && matches(bytes_per_dim, &lower, &upper, &doc_values[ord])
                        {
                            expected.insert(id);
                        }
                    }

                    let mut doc_id_to_id = MultiDocValues::get_numeric_values(&r, "id")?
                        .unwrap_or_else(|| {
                            randomizedtesting::jdk::exception::throw_without_message(
                                "java.lang.NullPointerException",
                            )
                        });

                    let mut fail_count = 0;
                    for doc_id in 0..r.max_doc() {
                        assert_equals(&doc_id, &doc_id_to_id.next_doc()?);
                        let id = doc_id_to_id.long_value()? as i32;
                        if hits.get(doc_id as usize) != expected.contains(&id) {
                            println!(
                                "FAIL: iter={iter} id={id} docID={doc_id} expected={} but got {} deleted?={} missing?={}",
                                expected.contains(&id),
                                hits.get(doc_id as usize),
                                deleted.contains(&id),
                                missing.contains(&id)
                            );
                            for dim in 0..num_dims {
                                println!(
                                    "  dim={dim} range: {} TO {}",
                                    bytes_to_string(Some(&lower[dim])),
                                    bytes_to_string(Some(&upper[dim]))
                                );
                                fail_count += 1;
                            }
                        }
                    }
                    if fail_count != 0 {
                        Assert::fail(&format!("{fail_count} hits were wrong"));
                    }
                    iter += 1;
                }
                Ok(())
            };
            if let Err(e) = run() {
                failed.store(true, Ordering::SeqCst);
                rethrow_java(e)
            }
        });
        thread.set_name(format!("T{i}"));
        thread.start();
        threads.push(thread);
    }

    starting_gun.count_down();
    for thread in &threads {
        let _ = thread.join();
    }

    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// `static String bytesToString(byte[] bytes)`.
fn bytes_to_string(bytes: Option<&[u8]>) -> String {
    match bytes {
        None => "null".to_string(),
        Some(bytes) => LuceneTestCase::new_bytes_ref_bytes(bytes).to_string(),
    }
}

/// `private static boolean matches(int bytesPerDim, byte[][] lower, byte[][] upper, byte[][] value)`.
fn matches(bytes_per_dim: usize, lower: &[Vec<u8>], upper: &[Vec<u8>], value: &[Vec<u8>]) -> bool {
    let num_dims = lower.len();
    for dim in 0..num_dims {
        if value[dim][..bytes_per_dim] < lower[dim][..bytes_per_dim] {
            // Value is below the lower bound, on this dim
            return false;
        }

        if value[dim][..bytes_per_dim] > upper[dim][..bytes_per_dim] {
            // Value is above the upper bound, on this dim
            return false;
        }
    }

    true
}

/// `private static long randomValue()`.
fn random_value() -> i64 {
    let value_range = VALUE_RANGE.load(Ordering::Relaxed);
    if value_range == 0 {
        random().next_long()
    } else {
        VALUE_MID
            .load(Ordering::Relaxed)
            .wrapping_add(i64::from(TestUtil::next_int(
                &*random(),
                -value_range,
                value_range,
            )))
    }
}

/// `newIndexWriterConfig()` with `setCodec(getCodec())`.
fn codec_config() -> Result<rucene::index::index_writer_config::IndexWriterConfig> {
    let iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_codec(get_codec())?;
    Ok(iwc)
}

/// `new RandomIndexWriter(random(), dir, iwc)` with [`codec_config`].
fn codec_random_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<RandomIndexWriter> {
    let iwc = codec_config()?;
    RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)
}

/// `IOUtils.close(r, w, dir)`.
fn close_rwd(
    r: Arc<dyn DirectoryReader>,
    w: RandomIndexWriter,
    mut dir: rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<()> {
    r.close()?;
    w.close()?;
    dir.close()
}

/// Port of `TestPointQueries.testMinMaxLong`.
fn test_min_max_long() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(LongPoint::new("value", &[i64::MIN])?)))?;
    w.add_document(&doc_with(Box::new(LongPoint::new("value", &[i64::MAX])?)))?;

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    assert_equals(
        &1,
        &s.count(LongPoint::new_range_query("value", &[i64::MIN], &[0])?)?,
    );
    assert_equals(
        &1,
        &s.count(LongPoint::new_range_query("value", &[0], &[i64::MAX])?)?,
    );
    assert_equals(
        &2,
        &s.count(LongPoint::new_range_query(
            "value",
            &[i64::MIN],
            &[i64::MAX],
        )?)?,
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// `private static byte[] toUTF8(String s)`.
fn to_utf8(s: &str) -> Vec<u8> {
    s.as_bytes().to_vec()
}

/// Right zero pads:
///
/// `private static byte[] toUTF8(String s, int length)`.
fn to_utf8_len(s: &str, length: usize) -> Result<Vec<u8>> {
    let bytes = s.as_bytes();
    if length < bytes.len() {
        return Err(LuceneError::illegal_argument(format!(
            "length={length} but string's UTF8 bytes has length={}",
            bytes.len()
        )));
    }
    let mut result = vec![0u8; length];
    result[..bytes.len()].copy_from_slice(bytes);
    Ok(result)
}

/// `BinaryPoint.newRangeQuery(String, byte[], byte[])`: the one-dimensional
/// `BinaryPoint.newRangeQuery(field, new byte[][] {lower}, new byte[][]
/// {upper})`.
fn binary_range(field: &str, lower: Vec<u8>, upper: Vec<u8>) -> Result<Arc<dyn Query>> {
    BinaryPoint::new_range_query(field, &[BytesRef::new(lower)], &[BytesRef::new(upper)])
}

/// `new BinaryPoint(name, value)` over a one-dimensional value.
fn binary_point(name: &str, value: Vec<u8>) -> Result<BinaryPoint> {
    BinaryPoint::new(name, &[BytesRef::new(value)])
}

/// Port of `TestPointQueries.testBasicSortedSet`.
fn test_basic_sorted_set() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(binary_point("value", to_utf8("abc"))?)))?;
    w.add_document(&doc_with(Box::new(binary_point("value", to_utf8("def"))?)))?;

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    assert_equals(
        &1,
        &s.count(binary_range("value", to_utf8("aaa"), to_utf8("bbb"))?)?,
    );
    assert_equals(
        &1,
        &s.count(binary_range(
            "value",
            to_utf8_len("c", 3)?,
            to_utf8_len("e", 3)?,
        )?)?,
    );
    assert_equals(
        &2,
        &s.count(binary_range(
            "value",
            to_utf8_len("a", 3)?,
            to_utf8_len("z", 3)?,
        )?)?,
    );
    assert_equals(
        &1,
        &s.count(binary_range("value", to_utf8_len("", 3)?, to_utf8("abc"))?)?,
    );
    assert_equals(
        &1,
        &s.count(binary_range("value", to_utf8_len("a", 3)?, to_utf8("abc"))?)?,
    );
    assert_equals(
        &0,
        &s.count(binary_range("value", to_utf8_len("a", 3)?, to_utf8("abb"))?)?,
    );
    assert_equals(
        &1,
        &s.count(binary_range("value", to_utf8("def"), to_utf8("zzz"))?)?,
    );
    assert_equals(
        &1,
        &s.count(binary_range("value", to_utf8("def"), to_utf8_len("z", 3)?)?)?,
    );
    assert_equals(
        &0,
        &s.count(binary_range("value", to_utf8("deg"), to_utf8_len("z", 3)?)?)?,
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// `testLongMinMaxNumeric` and `testLongMinMaxSortedSet` share their body.
fn long_min_max() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(LongPoint::new("value", &[i64::MIN])?)))?;
    w.add_document(&doc_with(Box::new(LongPoint::new("value", &[i64::MAX])?)))?;

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    let range = |a: i64, b: i64| LongPoint::new_range_query("value", &[a], &[b]);
    assert_equals(&2, &s.count(range(i64::MIN, i64::MAX)?)?);
    assert_equals(&1, &s.count(range(i64::MIN, i64::MAX - 1)?)?);
    assert_equals(&1, &s.count(range(i64::MIN + 1, i64::MAX)?)?);
    assert_equals(&0, &s.count(range(i64::MIN + 1, i64::MAX - 1)?)?);

    drop(s);
    close_rwd(r, w, dir)
}

/// Port of `TestPointQueries.testLongMinMaxNumeric`.
fn test_long_min_max_numeric() -> Result<()> {
    long_min_max()
}

/// Port of `TestPointQueries.testLongMinMaxSortedSet`.
fn test_long_min_max_sorted_set() -> Result<()> {
    long_min_max()
}

/// Port of `TestPointQueries.testSortedSetNoOrdsMatch`.
fn test_sorted_set_no_ords_match() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(binary_point("value", to_utf8("a"))?)))?;
    w.add_document(&doc_with(Box::new(binary_point("value", to_utf8("z"))?)))?;

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(
        &0,
        &s.count(binary_range("value", to_utf8("m"), to_utf8("m"))?)?,
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// Port of `TestPointQueries.testNumericNoValuesMatch`.
fn test_numeric_no_values_match() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(SortedNumericDocValuesField::new(
        "value", 17,
    ))))?;
    w.add_document(&doc_with(Box::new(SortedNumericDocValuesField::new(
        "value", 22,
    ))))?;

    let r = w.get_reader()?;

    let s = plain_searcher(&r)?;
    assert_equals(
        &0,
        &s.count(LongPoint::new_range_query("value", &[17], &[13])?)?,
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// Port of `TestPointQueries.testNoDocs`.
fn test_no_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&Document::new())?;

    let r = w.get_reader()?;

    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(
        &0,
        &s.count(LongPoint::new_range_query("value", &[17], &[13])?)?,
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// Port of `TestPointQueries.testWrongNumDims`.
fn test_wrong_num_dims() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(LongPoint::new("value", &[i64::MIN])?)))?;

    let r = w.get_reader()?;

    // no wrapping, else the exc might happen in executor thread:
    let s = plain_searcher(&r)?;
    let point = vec![BytesRef::new(vec![0u8; 8]), BytesRef::new(vec![0u8; 8])];
    let expected = expect_throws(IAE, || {
        s.count(BinaryPoint::new_range_query("value", &point, &point)?)
    });
    Assert::assert_equals_str(
        None,
        Some("field=\"value\" was indexed with numIndexDimensions=1 but this query has numDims=2"),
        expected.message(),
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// Port of `TestPointQueries.testWrongNumBytes`.
fn test_wrong_num_bytes() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = codec_random_writer(&dir)?;
    w.add_document(&doc_with(Box::new(LongPoint::new("value", &[i64::MIN])?)))?;

    let r = w.get_reader()?;

    // no wrapping, else the exc might happen in executor thread:
    let s = plain_searcher(&r)?;
    let point = vec![BytesRef::new(vec![0u8; 10])];
    let expected = expect_throws(IAE, || {
        s.count(BinaryPoint::new_range_query("value", &point, &point)?)
    });
    Assert::assert_equals_str(
        None,
        Some("field=\"value\" was indexed with bytesPerDim=8 but this query has bytesPerDim=10"),
        expected.message(),
    );

    drop(s);
    close_rwd(r, w, dir)
}

/// A document with `id:0` and `value:0L`.
fn id_zero_doc() -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(StringField::new(
        "id",
        "0".to_string(),
        Store::NO,
    )?));
    doc.add(Box::new(LongPoint::new("value", &[0])?));
    Ok(doc)
}

/// Port of `TestPointQueries.testAllPointDocsWereDeletedAndThenMergedAgain`.
fn test_all_point_docs_were_deleted_and_then_merged_again() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;
    w.add_document(&id_zero_doc()?)?;

    // Add document that won't be deleted to avoid IW dropping
    // segment below since it's 100% deleted:
    w.add_document(&Document::new())?;
    w.commit()?;

    // Need another segment so we invoke BKDWriter.merge
    w.add_document(&id_zero_doc()?)?;
    w.add_document(&Document::new())?;

    w.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;
    w.force_merge(1)?;

    w.add_document(&id_zero_doc()?)?;
    w.add_document(&Document::new())?;

    w.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;
    w.force_merge(1)?;

    w.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `PointsFormat` of `getCodec()`: a `Lucene90PointsWriter`
/// with the drawn parameters, and the stock reader.
#[derive(Debug)]
struct RandomizedPointsFormat {
    max_points_in_leaf_node: i32,
    max_mb_sort_in_heap: f64,
}

impl PointsFormat for RandomizedPointsFormat {
    fn name(&self) -> &str {
        "Lucene90"
    }

    fn fields_writer(&self, write_state: &SegmentWriteState) -> Result<Box<dyn PointsWriter>> {
        Ok(Box::new(Lucene90PointsWriter::with_config(
            write_state,
            self.max_points_in_leaf_node,
            self.max_mb_sort_in_heap,
            VERSION_CURRENT,
        )?))
    }

    fn fields_reader(&self, read_state: &SegmentReadState) -> Result<Box<dyn PointsReader>> {
        Ok(Box::new(Lucene90PointsReader::new(read_state)?))
    }
}

/// `private static Codec getCodec()`.
fn get_codec() -> Arc<dyn Codec> {
    let default = rucene::codecs::get_default().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.lang.NullPointerException")
    });
    if default.name() == "Lucene84" {
        let max_points_in_leaf_node = TestUtil::next_int(&*random(), 16, 2048);
        let max_mb_sort_in_heap = 5.0 + (3.0 * random().next_double());
        if verbose() {
            println!(
                "TEST: using Lucene60PointsFormat with maxPointsInLeafNode={max_points_in_leaf_node} and maxMBSortInHeap={max_mb_sort_in_heap}"
            );
        }

        Arc::new(
            FilterCodec::new("Lucene84", default).with_points_format(RandomizedPointsFormat {
                max_points_in_leaf_node,
                max_mb_sort_in_heap,
            }),
        )
    } else {
        default
    }
}

/// Port of `TestPointQueries.testExactPoints`.
fn test_exact_points() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    w.add_document(&doc_with(Box::new(LongPoint::new("long", &[5])?)))?;
    w.add_document(&doc_with(Box::new(IntPoint::new("int", &[42])?)))?;
    w.add_document(&doc_with(Box::new(FloatPoint::new("float", &[2.0])?)))?;
    w.add_document(&doc_with(Box::new(DoublePoint::new("double", &[1.0])?)))?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(&1, &s.count(IntPoint::new_exact_query("int", 42)?)?);
    assert_equals(&0, &s.count(IntPoint::new_exact_query("int", 41)?)?);

    assert_equals(&1, &s.count(LongPoint::new_exact_query("long", 5)?)?);
    assert_equals(&0, &s.count(LongPoint::new_exact_query("long", -1)?)?);

    assert_equals(&1, &s.count(FloatPoint::new_exact_query("float", 2.0)?)?);
    assert_equals(&0, &s.count(FloatPoint::new_exact_query("float", 1.0)?)?);

    assert_equals(&1, &s.count(DoublePoint::new_exact_query("double", 1.0)?)?);
    assert_equals(&0, &s.count(DoublePoint::new_exact_query("double", 2.0)?)?);
    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// `query.toString()`.
fn to_s(query: Result<Arc<dyn Query>>) -> Result<String> {
    Ok(query_to_string(&*query?))
}

/// Port of `TestPointQueries.testToString`.
fn test_to_string() -> Result<()> {
    // ints
    assert_equals(
        "field:[1 TO 2]",
        to_s(IntPoint::new_range_query("field", &[1], &[2]))?.as_str(),
    );
    assert_equals(
        "field:[-2 TO 1]",
        to_s(IntPoint::new_range_query("field", &[-2], &[1]))?.as_str(),
    );

    // longs
    assert_equals(
        "field:[1099511627776 TO 2199023255552]",
        to_s(LongPoint::new_range_query(
            "field",
            &[1i64 << 40],
            &[1i64 << 41],
        ))?
        .as_str(),
    );
    assert_equals(
        "field:[-5 TO 6]",
        to_s(LongPoint::new_range_query("field", &[-5], &[6]))?.as_str(),
    );

    // floats
    assert_equals(
        "field:[1.3 TO 2.5]",
        to_s(FloatPoint::new_range_query("field", &[1.3], &[2.5]))?.as_str(),
    );
    assert_equals(
        "field:[-2.9 TO 1.0]",
        to_s(FloatPoint::new_range_query("field", &[-2.9], &[1.0]))?.as_str(),
    );

    // doubles
    assert_equals(
        "field:[1.3 TO 2.5]",
        to_s(DoublePoint::new_range_query("field", &[1.3], &[2.5]))?.as_str(),
    );
    assert_equals(
        "field:[-2.9 TO 1.0]",
        to_s(DoublePoint::new_range_query("field", &[-2.9], &[1.0]))?.as_str(),
    );

    // n-dimensional double
    assert_equals(
        "field:[1.3 TO 2.5],[-2.9 TO 1.0]",
        to_s(DoublePoint::new_range_query(
            "field",
            &[1.3, -2.9],
            &[2.5, 1.0],
        ))?
        .as_str(),
    );
    Ok(())
}

/// `private int[] toArray(Set<Integer> valuesSet)`.
fn to_array(values_set: &JavaHashSet<i32>) -> Vec<i32> {
    let mut values = vec![0i32; values_set.size()];
    let mut upto = 0;
    for value in values_set.iter() {
        values[upto] = *value;
        upto += 1;
    }
    values
}

/// `private static int randomIntValue(Integer min, Integer max)`.
fn random_int_value(min: Option<i32>, max: Option<i32>) -> i32 {
    match min {
        None => random().next_int(),
        Some(min) => TestUtil::next_int(&*random(), min, max.expect("max set with min")),
    }
}

/// Port of `TestPointQueries.testRandomPointInSetQuery`.
fn test_random_point_in_set_query() -> Result<()> {
    let use_narrow_range = random().next_boolean();
    let value_min: Option<i32>;
    let value_max: Option<i32>;
    let num_values;
    if use_narrow_range {
        let gap = random().next_int_bound(100);
        let min = random().next_int_bound(i32::MAX - gap);
        value_min = Some(min);
        value_max = Some(min + gap);
        num_values = TestUtil::next_int(&*random(), 1, gap + 1);
    } else {
        value_min = None;
        value_max = None;
        num_values = TestUtil::next_int(&*random(), 1, 100);
    }
    let mut values_set: JavaHashSet<i32> = JavaHashSet::new();
    while (values_set.size() as i32) < num_values {
        values_set.add(random_int_value(value_min, value_max));
    }
    let values = Arc::new(to_array(&values_set));
    let num_docs = TestUtil::next_int(&*random(), 1, 10000);

    if verbose() {
        println!("TEST: numValues={num_values} numDocs={num_docs}");
    }

    let mut dir = if num_docs > 100000 {
        LuceneTestCase::new_fs_directory(&LuceneTestCase::create_temp_dir_prefix(
            "TestPointQueries",
        ))
    } else {
        LuceneTestCase::new_directory()
    };

    let iwc = codec_config()?;
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;

    let mut doc_values = vec![0i32; num_docs as usize];
    for i in 0..num_docs as usize {
        let x = values[random().next_int_bound(values.len() as i32) as usize];
        let doc = doc_with(Box::new(IntPoint::new("int", &[x])?));
        doc_values[i] = x;
        w.add_document(&doc)?;
    }
    let doc_values = Arc::new(doc_values);

    if random().next_boolean() {
        if verbose() {
            println!("  forceMerge(1)");
        }
        w.force_merge(1)?;
    }
    let r = w.get_reader()?;
    w.close()?;

    let s = Arc::new(LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false));

    let num_threads = TestUtil::next_int(&*random(), 2, 5);

    if verbose() {
        println!("TEST: use {num_threads} query threads; searcher={s:?}");
    }

    let mut threads = Vec::new();
    let iters = LuceneTestCase::at_least(100);

    let starting_gun = Arc::new(crate::core_search::support::CountDownLatch::new(1));
    let failed = Arc::new(AtomicBool::new(false));

    for i in 0..num_threads {
        let starting_gun = Arc::clone(&starting_gun);
        let failed = Arc::clone(&failed);
        let s = Arc::clone(&s);
        let values = Arc::clone(&values);
        let doc_values = Arc::clone(&doc_values);
        let thread = Thread::unnamed(move || {
            let run = || -> Result<()> {
                starting_gun.await_();

                let mut iter = 0;
                while iter < iters && !failed.load(Ordering::SeqCst) {
                    let num_valid_values_to_query = random().next_int_bound(values.len() as i32);

                    let mut values_to_query: JavaHashSet<i32> = JavaHashSet::new();
                    while (values_to_query.size() as i32) < num_valid_values_to_query {
                        values_to_query
                            .add(values[random().next_int_bound(values.len() as i32) as usize]);
                    }

                    let num_extra_values_to_query = random().next_int_bound(20);
                    while (values_to_query.size() as i32)
                        < num_valid_values_to_query + num_extra_values_to_query
                    {
                        values_to_query.add(random().next_int());
                    }

                    let mut expected_count = 0;
                    for value in doc_values.iter() {
                        if values_to_query.contains(value) {
                            expected_count += 1;
                        }
                    }

                    if verbose() {
                        println!(
                            "TEST: thread={} values={:?} expectedCount={expected_count}",
                            current_thread_name(),
                            values_to_query.to_vec()
                        );
                    }

                    assert_equals(
                        &expected_count,
                        &s.count(IntPoint::new_set_query("int", &to_array(&values_to_query))?)?,
                    );
                    iter += 1;
                }
                Ok(())
            };
            if let Err(e) = run() {
                failed.store(true, Ordering::SeqCst);
                rethrow_java(e)
            }
        });
        thread.set_name(format!("T{i}"));
        thread.start();
        threads.push(thread);
    }
    starting_gun.count_down();
    for thread in &threads {
        let _ = thread.join();
    }
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// The `toString(byte[])` of the anonymous `PointInSetQuery` of
/// `newMultiDimIntSetQuery`: the decoded dimensions, comma-separated.
#[derive(Debug)]
struct MultiDimIntFormatter {
    num_dims: usize,
}

impl PointFormatter for MultiDimIntFormatter {
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == self.num_dims * 4);
        let mut sb = String::new();
        for dim in 0..self.num_dims {
            if dim > 0 {
                sb.push(',');
            }
            sb.push_str(&IntPoint::decode_dimension(value, dim * 4).to_string());
        }
        sb
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The anonymous `PointInSetQuery.Stream` of `newMultiDimIntSetQuery`: the
/// sorted packed values, in order.
struct PackedValuesStream {
    packed_values: Vec<Vec<u8>>,
    upto: usize,
}

impl BytesRefIterator for PackedValuesStream {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.upto >= self.packed_values.len() {
            return Ok(None);
        }
        let value = BytesRef::new(self.packed_values[self.upto].clone());
        self.upto += 1;
        Ok(Some(value))
    }
}

// TODO: in the future, if there is demand for real usage, we can "graduate" this test-only query
// factory as IntPoint.newMultiSetQuery or
// something (and same for other XXXPoint classes):
/// `private static Query newMultiDimIntSetQuery(String field, final int numDims, int... valuesIn)`.
fn new_multi_dim_int_set_query(
    field: &str,
    num_dims: usize,
    values_in: &[i32],
) -> Result<Arc<dyn Query>> {
    if values_in.len() % num_dims != 0 {
        return Err(LuceneError::illegal_argument(format!(
            "incongruent number of values: valuesIn.length={} but numDims={num_dims}",
            values_in.len()
        )));
    }

    // Pack all values:
    let mut packed_values: Vec<Vec<u8>> = Vec::with_capacity(values_in.len() / num_dims);
    for i in 0..values_in.len() / num_dims {
        let mut packed_value = vec![0u8; num_dims * 4];
        for dim in 0..num_dims {
            IntPoint::encode_dimension(values_in[i * num_dims + dim], &mut packed_value, dim * 4);
        }
        packed_values.push(packed_value);
    }

    // Sort:
    packed_values.sort_by(|a, b| a.as_slice().cmp(b.as_slice()));

    let mut stream = PackedValuesStream {
        packed_values,
        upto: 0,
    };
    Ok(Arc::new(PointInSetQuery::new(
        field,
        num_dims as i32,
        4,
        &mut stream,
        Arc::new(MultiDimIntFormatter { num_dims }),
    )?))
}

/// Port of `TestPointQueries.testBasicMultiDimPointInSetQuery`.
fn test_basic_multi_dim_point_in_set_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    w.add_document(&doc_with(Box::new(IntPoint::new("int", &[17, 42])?)))?;
    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    assert_equals(
        &0,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 41])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 42])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[-7, -7, 17, 42])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 42, -14, -14])?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testBasicMultiValueMultiDimPointInSetQuery`.
fn test_basic_multi_value_multi_dim_point_in_set_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[17, 42])?));
    doc.add(Box::new(IntPoint::new("int", &[34, 79])?));
    w.add_document(&doc)?;
    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    assert_equals(
        &0,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 41])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 42])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 42, 34, 79])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[-7, -7, 17, 42])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[-7, -7, 34, 79])?)?,
    );
    assert_equals(
        &1,
        &s.count(new_multi_dim_int_set_query("int", 2, &[17, 42, -14, -14])?)?,
    );

    assert_equals(
        "int:{-14,-14 17,42}",
        to_s(new_multi_dim_int_set_query("int", 2, &[17, 42, -14, -14]))?.as_str(),
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testManyEqualValuesMultiDimPointInSetQuery`.
fn test_many_equal_values_multi_dim_point_in_set_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut zero_count = 0;
    for _i in 0..10000 {
        let x = random().next_int_bound(2);
        if x == 0 {
            zero_count += 1;
        }
        w.add_document(&doc_with(Box::new(IntPoint::new("int", &[x, x])?)))?;
    }
    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    assert_equals(
        &zero_count,
        &s.count(new_multi_dim_int_set_query("int", 2, &[0, 0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(new_multi_dim_int_set_query("int", 2, &[1, 1])?)?,
    );
    assert_equals(
        &0,
        &s.count(new_multi_dim_int_set_query("int", 2, &[2, 2])?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testInvalidMultiDimPointInSetQuery`.
fn test_invalid_multi_dim_point_in_set_query() -> Result<()> {
    let expected = expect_throws(IAE, || new_multi_dim_int_set_query("int", 2, &[3, 4, 5]));
    Assert::assert_equals_str(
        None,
        Some("incongruent number of values: valuesIn.length=3 but numDims=2"),
        expected.message(),
    );
    Ok(())
}

/// A document with an `IntPoint`, `LongPoint`, `FloatPoint`, `DoublePoint`
/// and `BinaryPoint {0, x}` of value `x`.
fn all_points_doc(doc: &mut Document, x: i32) -> Result<()> {
    doc.add(Box::new(IntPoint::new("int", &[x])?));
    doc.add(Box::new(LongPoint::new("long", &[i64::from(x)])?));
    doc.add(Box::new(FloatPoint::new("float", &[x as f32])?));
    doc.add(Box::new(DoublePoint::new("double", &[f64::from(x)])?));
    doc.add(Box::new(binary_point("bytes", vec![0, x as u8])?));
    Ok(())
}

/// `(byte) v`.
fn b(v: i32) -> u8 {
    v as u8
}

/// Port of `TestPointQueries.testBasicPointInSetQuery`.
fn test_basic_point_in_set_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    for x in [17, 42, 97] {
        let mut doc = Document::new();
        all_points_doc(&mut doc, x)?;
        w.add_document(&doc)?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(&0, &s.count(IntPoint::new_set_query("int", &[16])?)?);
    assert_equals(&1, &s.count(IntPoint::new_set_query("int", &[17])?)?);
    assert_equals(
        &3,
        &s.count(IntPoint::new_set_query("int", &[17, 97, 42])?)?,
    );
    assert_equals(
        &3,
        &s.count(IntPoint::new_set_query("int", &[-7, 17, 42, 97])?)?,
    );
    assert_equals(
        &3,
        &s.count(IntPoint::new_set_query("int", &[17, 20, 42, 97])?)?,
    );
    assert_equals(
        &3,
        &s.count(IntPoint::new_set_query("int", &[17, 105, 42, 97])?)?,
    );

    assert_equals(&0, &s.count(LongPoint::new_set_query("long", &[16])?)?);
    assert_equals(&1, &s.count(LongPoint::new_set_query("long", &[17])?)?);
    assert_equals(
        &3,
        &s.count(LongPoint::new_set_query("long", &[17, 97, 42])?)?,
    );
    assert_equals(
        &3,
        &s.count(LongPoint::new_set_query("long", &[-7, 17, 42, 97])?)?,
    );
    assert_equals(
        &3,
        &s.count(LongPoint::new_set_query("long", &[17, 20, 42, 97])?)?,
    );
    assert_equals(
        &3,
        &s.count(LongPoint::new_set_query("long", &[17, 105, 42, 97])?)?,
    );

    assert_equals(&0, &s.count(FloatPoint::new_set_query("float", &[16.0])?)?);
    assert_equals(&1, &s.count(FloatPoint::new_set_query("float", &[17.0])?)?);
    assert_equals(
        &3,
        &s.count(FloatPoint::new_set_query("float", &[17.0, 97.0, 42.0])?)?,
    );
    assert_equals(
        &3,
        &s.count(FloatPoint::new_set_query(
            "float",
            &[-7.0, 17.0, 42.0, 97.0],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(FloatPoint::new_set_query(
            "float",
            &[17.0, 20.0, 42.0, 97.0],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(FloatPoint::new_set_query(
            "float",
            &[17.0, 105.0, 42.0, 97.0],
        )?)?,
    );

    assert_equals(
        &0,
        &s.count(DoublePoint::new_set_query("double", &[16.0])?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_set_query("double", &[17.0])?)?,
    );
    assert_equals(
        &3,
        &s.count(DoublePoint::new_set_query("double", &[17.0, 97.0, 42.0])?)?,
    );
    assert_equals(
        &3,
        &s.count(DoublePoint::new_set_query(
            "double",
            &[-7.0, 17.0, 42.0, 97.0],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(DoublePoint::new_set_query(
            "double",
            &[17.0, 20.0, 42.0, 97.0],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(DoublePoint::new_set_query(
            "double",
            &[17.0, 105.0, 42.0, 97.0],
        )?)?,
    );

    assert_equals(
        &0,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![0, 16]])?)?,
    );
    assert_equals(
        &1,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![0, 17]])?)?,
    );
    assert_equals(
        &3,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, 17], vec![0, 97], vec![0, 42]],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, b(-7)], vec![0, 17], vec![0, 42], vec![0, 97]],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, 17], vec![0, 20], vec![0, 42], vec![0, 97]],
        )?)?,
    );
    assert_equals(
        &3,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, 17], vec![0, 105], vec![0, 42], vec![0, 97]],
        )?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// `assertEquals(q1, q2)` over `Query.equals`.
fn assert_query_equals(expected: &Arc<dyn Query>, actual: &Arc<dyn Query>) {
    if !expected.query_eq(&**actual) {
        Assert::fail(&format!(
            "expected:<{}> but was:<{}>",
            query_to_string(&**expected),
            query_to_string(&**actual)
        ));
    }
}

/// Boxed methods for primitive types should behave the same as unboxed: just sugar
///
/// Port of `TestPointQueries.testPointIntSetBoxed`.
fn test_point_int_set_boxed() -> Result<()> {
    assert_query_equals(
        &IntPoint::new_set_query("foo", &[1, 2, 3])?,
        &IntPoint::new_set_query_collection("foo", vec![1, 2, 3])?,
    );
    assert_query_equals(
        &FloatPoint::new_set_query("foo", &[1.0, 2.0, 3.0])?,
        &FloatPoint::new_set_query_collection("foo", vec![1.0, 2.0, 3.0])?,
    );
    assert_query_equals(
        &LongPoint::new_set_query("foo", &[1, 2, 3])?,
        &LongPoint::new_set_query_collection("foo", vec![1, 2, 3])?,
    );
    assert_query_equals(
        &DoublePoint::new_set_query("foo", &[1.0, 2.0, 3.0])?,
        &DoublePoint::new_set_query_collection("foo", vec![1.0, 2.0, 3.0])?,
    );
    Ok(())
}

/// Port of `TestPointQueries.testBasicMultiValuedPointInSetQuery`.
fn test_basic_multi_valued_point_in_set_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[17])?));
    doc.add(Box::new(IntPoint::new("int", &[42])?));
    doc.add(Box::new(LongPoint::new("long", &[17])?));
    doc.add(Box::new(LongPoint::new("long", &[42])?));
    doc.add(Box::new(FloatPoint::new("float", &[17.0])?));
    doc.add(Box::new(FloatPoint::new("float", &[42.0])?));
    doc.add(Box::new(DoublePoint::new("double", &[17.0])?));
    doc.add(Box::new(DoublePoint::new("double", &[42.0])?));
    doc.add(Box::new(binary_point("bytes", vec![0, 17])?));
    doc.add(Box::new(binary_point("bytes", vec![0, 42])?));
    w.add_document(&doc)?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(&0, &s.count(IntPoint::new_set_query("int", &[16])?)?);
    assert_equals(&1, &s.count(IntPoint::new_set_query("int", &[17])?)?);
    assert_equals(
        &1,
        &s.count(IntPoint::new_set_query("int", &[17, 97, 42])?)?,
    );
    assert_equals(
        &1,
        &s.count(IntPoint::new_set_query("int", &[-7, 17, 42, 97])?)?,
    );
    assert_equals(
        &0,
        &s.count(IntPoint::new_set_query("int", &[16, 20, 41, 97])?)?,
    );

    assert_equals(&0, &s.count(LongPoint::new_set_query("long", &[16])?)?);
    assert_equals(&1, &s.count(LongPoint::new_set_query("long", &[17])?)?);
    assert_equals(
        &1,
        &s.count(LongPoint::new_set_query("long", &[17, 97, 42])?)?,
    );
    assert_equals(
        &1,
        &s.count(LongPoint::new_set_query("long", &[-7, 17, 42, 97])?)?,
    );
    assert_equals(
        &0,
        &s.count(LongPoint::new_set_query("long", &[16, 20, 41, 97])?)?,
    );

    assert_equals(&0, &s.count(FloatPoint::new_set_query("float", &[16.0])?)?);
    assert_equals(&1, &s.count(FloatPoint::new_set_query("float", &[17.0])?)?);
    assert_equals(
        &1,
        &s.count(FloatPoint::new_set_query("float", &[17.0, 97.0, 42.0])?)?,
    );
    assert_equals(
        &1,
        &s.count(FloatPoint::new_set_query(
            "float",
            &[-7.0, 17.0, 42.0, 97.0],
        )?)?,
    );
    assert_equals(
        &0,
        &s.count(FloatPoint::new_set_query(
            "float",
            &[16.0, 20.0, 41.0, 97.0],
        )?)?,
    );

    assert_equals(
        &0,
        &s.count(DoublePoint::new_set_query("double", &[16.0])?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_set_query("double", &[17.0])?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_set_query("double", &[17.0, 97.0, 42.0])?)?,
    );
    assert_equals(
        &1,
        &s.count(DoublePoint::new_set_query(
            "double",
            &[-7.0, 17.0, 42.0, 97.0],
        )?)?,
    );
    assert_equals(
        &0,
        &s.count(DoublePoint::new_set_query(
            "double",
            &[16.0, 20.0, 41.0, 97.0],
        )?)?,
    );

    assert_equals(
        &0,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![0, 16]])?)?,
    );
    assert_equals(
        &1,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![0, 17]])?)?,
    );
    assert_equals(
        &1,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, 17], vec![0, 97], vec![0, 42]],
        )?)?,
    );
    assert_equals(
        &1,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, b(-7)], vec![0, 17], vec![0, 42], vec![0, 97]],
        )?)?,
    );
    assert_equals(
        &0,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0, 16], vec![0, 20], vec![0, 41], vec![0, 97]],
        )?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testEmptyPointInSetQuery`.
fn test_empty_point_in_set_query() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut doc = Document::new();
    all_points_doc(&mut doc, 17)?;
    w.add_document(&doc)?;

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(&0, &s.count(IntPoint::new_set_query("int", &[])?)?);
    assert_equals(&0, &s.count(LongPoint::new_set_query("long", &[])?)?);
    assert_equals(&0, &s.count(FloatPoint::new_set_query("float", &[])?)?);
    assert_equals(&0, &s.count(DoublePoint::new_set_query("double", &[])?)?);
    assert_equals(&0, &s.count(BinaryPoint::new_set_query("bytes", &[])?)?);

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// A document with every point kind of value `x`, the binary one being the
/// single byte `(byte) x`.
fn all_points_single_byte_doc(x: i32) -> Result<Document> {
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("int", &[x])?));
    doc.add(Box::new(LongPoint::new("long", &[i64::from(x)])?));
    doc.add(Box::new(FloatPoint::new("float", &[x as f32])?));
    doc.add(Box::new(DoublePoint::new("double", &[f64::from(x)])?));
    doc.add(Box::new(binary_point("bytes", vec![x as u8])?));
    Ok(doc)
}

/// Port of `TestPointQueries.testPointInSetQueryManyEqualValues`.
fn test_point_in_set_query_many_equal_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut zero_count = 0;
    for _i in 0..10000 {
        let x = random().next_int_bound(2);
        if x == 0 {
            zero_count += 1;
        }
        w.add_document(&all_points_single_byte_doc(x)?)?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_set_query("int", &[0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_set_query("int", &[0, -7])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_set_query("int", &[7, 0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(IntPoint::new_set_query("int", &[1])?)?,
    );
    assert_equals(&0, &s.count(IntPoint::new_set_query("int", &[2])?)?);

    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_set_query("long", &[0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_set_query("long", &[0, -7])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_set_query("long", &[7, 0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(LongPoint::new_set_query("long", &[1])?)?,
    );
    assert_equals(&0, &s.count(LongPoint::new_set_query("long", &[2])?)?);

    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_set_query("float", &[0.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_set_query("float", &[0.0, -7.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_set_query("float", &[7.0, 0.0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(FloatPoint::new_set_query("float", &[1.0])?)?,
    );
    assert_equals(&0, &s.count(FloatPoint::new_set_query("float", &[2.0])?)?);

    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_set_query("double", &[0.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_set_query("double", &[0.0, -7.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_set_query("double", &[7.0, 0.0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(DoublePoint::new_set_query("double", &[1.0])?)?,
    );
    assert_equals(&0, &s.count(DoublePoint::new_set_query("double", &[2.0])?)?);

    assert_equals(
        &zero_count,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![0]])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0], vec![b(-7)]],
        )?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![7], vec![0]])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![1]])?)?,
    );
    assert_equals(
        &0,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![2]])?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testPointRangeQueryManyEqualValues`.
fn test_point_range_query_many_equal_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let cardinality = TestUtil::next_int(&*random(), 2, 20);

    let mut zero_count = 0;
    let mut one_count = 0;
    for _i in 0..10000 {
        let x = random().next_int_bound(cardinality);
        if x == 0 {
            zero_count += 1;
        } else if x == 1 {
            one_count += 1;
        }
        w.add_document(&all_points_single_byte_doc(x)?)?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);

    let rest = 10000 - zero_count - one_count;

    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_range_query("int", &[0], &[0])?)?,
    );
    assert_equals(
        &one_count,
        &s.count(IntPoint::new_range_query("int", &[1], &[1])?)?,
    );
    assert_equals(
        &(zero_count + one_count),
        &s.count(IntPoint::new_range_query("int", &[0], &[1])?)?,
    );
    assert_equals(
        &rest,
        &s.count(IntPoint::new_range_query("int", &[2], &[cardinality])?)?,
    );

    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_range_query("long", &[0], &[0])?)?,
    );
    assert_equals(
        &one_count,
        &s.count(LongPoint::new_range_query("long", &[1], &[1])?)?,
    );
    assert_equals(
        &(zero_count + one_count),
        &s.count(LongPoint::new_range_query("long", &[0], &[1])?)?,
    );
    assert_equals(
        &rest,
        &s.count(LongPoint::new_range_query(
            "long",
            &[2],
            &[i64::from(cardinality)],
        )?)?,
    );

    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_range_query("float", &[0.0], &[0.0])?)?,
    );
    assert_equals(
        &one_count,
        &s.count(FloatPoint::new_range_query("float", &[1.0], &[1.0])?)?,
    );
    assert_equals(
        &(zero_count + one_count),
        &s.count(FloatPoint::new_range_query("float", &[0.0], &[1.0])?)?,
    );
    assert_equals(
        &rest,
        &s.count(FloatPoint::new_range_query(
            "float",
            &[2.0],
            &[cardinality as f32],
        )?)?,
    );

    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_range_query("double", &[0.0], &[0.0])?)?,
    );
    assert_equals(
        &one_count,
        &s.count(DoublePoint::new_range_query("double", &[1.0], &[1.0])?)?,
    );
    assert_equals(
        &(zero_count + one_count),
        &s.count(DoublePoint::new_range_query("double", &[0.0], &[1.0])?)?,
    );
    assert_equals(
        &rest,
        &s.count(DoublePoint::new_range_query(
            "double",
            &[2.0],
            &[f64::from(cardinality)],
        )?)?,
    );

    assert_equals(
        &zero_count,
        &s.count(binary_range("bytes", vec![0], vec![0])?)?,
    );
    assert_equals(
        &one_count,
        &s.count(binary_range("bytes", vec![1], vec![1])?)?,
    );
    assert_equals(
        &(zero_count + one_count),
        &s.count(binary_range("bytes", vec![0], vec![1])?)?,
    );
    assert_equals(
        &rest,
        &s.count(binary_range("bytes", vec![2], vec![cardinality as u8])?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testPointInSetQueryManyEqualValuesWithBigGap`.
fn test_point_in_set_query_many_equal_values_with_big_gap() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = codec_config()?;
    let w = IndexWriter::new(dir.as_directory(), iwc)?;

    let mut zero_count = 0;
    for _i in 0..10000 {
        let x = 200 * random().next_int_bound(2);
        if x == 0 {
            zero_count += 1;
        }
        w.add_document(&all_points_single_byte_doc(x)?)?;
    }

    let r = directory_reader::open_with_writer(&*w)?;
    let s = LuceneTestCase::new_searcher_wrap(Arc::clone(&r), false);
    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_set_query("int", &[0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_set_query("int", &[0, -7])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(IntPoint::new_set_query("int", &[7, 0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(IntPoint::new_set_query("int", &[200])?)?,
    );
    assert_equals(&0, &s.count(IntPoint::new_set_query("int", &[2])?)?);

    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_set_query("long", &[0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_set_query("long", &[0, -7])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(LongPoint::new_set_query("long", &[7, 0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(LongPoint::new_set_query("long", &[200])?)?,
    );
    assert_equals(&0, &s.count(LongPoint::new_set_query("long", &[2])?)?);

    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_set_query("float", &[0.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_set_query("float", &[0.0, -7.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(FloatPoint::new_set_query("float", &[7.0, 0.0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(FloatPoint::new_set_query("float", &[200.0])?)?,
    );
    assert_equals(&0, &s.count(FloatPoint::new_set_query("float", &[2.0])?)?);

    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_set_query("double", &[0.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_set_query("double", &[0.0, -7.0])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(DoublePoint::new_set_query("double", &[7.0, 0.0])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(DoublePoint::new_set_query("double", &[200.0])?)?,
    );
    assert_equals(&0, &s.count(DoublePoint::new_set_query("double", &[2.0])?)?);

    assert_equals(
        &zero_count,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![0]])?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(BinaryPoint::new_set_query(
            "bytes",
            &[vec![0], vec![b(-7)]],
        )?)?,
    );
    assert_equals(
        &zero_count,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![7], vec![0]])?)?,
    );
    assert_equals(
        &(10000 - zero_count),
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![b(200)]])?)?,
    );
    assert_equals(
        &0,
        &s.count(BinaryPoint::new_set_query("bytes", &[vec![2]])?)?,
    );

    w.close()?;
    drop(s);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// The `toString(byte[])` of the anonymous `PointInSetQuery`s of
/// `testInvalidPointInSetQuery` and `testOutOfOrderValuesInPointInSetQuery`:
/// `Arrays.toString(point)`.
#[derive(Debug)]
struct ArraysToStringFormatter;

impl PointFormatter for ArraysToStringFormatter {
    fn format(&self, value: &[u8]) -> String {
        java_bytes_to_string(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// The anonymous `PointInSetQuery.Stream` of `testInvalidPointInSetQuery`:
/// `newBytesRef(new byte[3])` on every call.
struct ThreeZeroBytesStream;

impl BytesRefIterator for ThreeZeroBytesStream {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        Ok(Some(LuceneTestCase::new_bytes_ref_bytes(&[0u8; 3])))
    }
}

/// Port of `TestPointQueries.testInvalidPointInSetQuery`.
fn test_invalid_point_in_set_query() -> Result<()> {
    let expected = expect_throws(IAE, || {
        PointInSetQuery::new(
            "foo",
            3,
            4,
            &mut ThreeZeroBytesStream,
            Arc::new(ArraysToStringFormatter),
        )
    });
    Assert::assert_equals_str(
        None,
        Some("packed point length should be 12 but got 3; field=\"foo\" numDims=3 bytesPerDim=4"),
        expected.message(),
    );
    Ok(())
}

/// Port of `TestPointQueries.testInvalidPointInSetBinaryQuery`.
fn test_invalid_point_in_set_binary_query() -> Result<()> {
    let expected = expect_throws(IAE, || {
        BinaryPoint::new_set_query("bytes", &[vec![2], vec![]])
    });
    Assert::assert_equals_str(
        None,
        Some("all byte[] must be the same length, but saw 1 and 0"),
        expected.message(),
    );
    Ok(())
}

/// Port of `TestPointQueries.testPointInSetQueryToString`.
fn test_point_in_set_query_to_string() -> Result<()> {
    // int
    assert_equals(
        "int:{-42 18}",
        to_s(IntPoint::new_set_query("int", &[-42, 18]))?.as_str(),
    );

    // long
    assert_equals(
        "long:{-42 18}",
        to_s(LongPoint::new_set_query("long", &[-42, 18]))?.as_str(),
    );

    // float
    assert_equals(
        "float:{-42.0 18.0}",
        to_s(FloatPoint::new_set_query("float", &[-42.0, 18.0]))?.as_str(),
    );

    // double
    assert_equals(
        "double:{-42.0 18.0}",
        to_s(DoublePoint::new_set_query("double", &[-42.0, 18.0]))?.as_str(),
    );

    // binary
    assert_equals(
        "bytes:{[12] [2a]}",
        to_s(BinaryPoint::new_set_query("bytes", &[vec![42], vec![18]]))?.as_str(),
    );
    Ok(())
}

/// `Iterator.next()` of the collection `getPackedPoints()` returns: the next
/// element, or `java.util.NoSuchElementException` past the end.
fn java_next(iterator: &mut std::vec::IntoIter<Vec<u8>>) -> Vec<u8> {
    iterator.next().unwrap_or_else(|| {
        randomizedtesting::jdk::exception::throw_without_message("java.util.NoSuchElementException")
    })
}

/// Port of `TestPointQueries.testPointInSetQueryGetPackedPoints`.
fn test_point_in_set_query_get_packed_points() -> Result<()> {
    let num_values = random_int_value(Some(1), Some(32));
    let mut values: Vec<Vec<u8>> = Vec::with_capacity(num_values as usize);
    let mut i: u8 = 0;
    while i32::from(i) < num_values {
        values.push(vec![i]);
        i += 1;
    }

    let query = BinaryPoint::new_set_query("field", &values)?;
    let query = query
        .as_any()
        .downcast_ref::<PointInSetQuery>()
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message("java.lang.ClassCastException")
        });
    let packed_points = query.get_packed_points()?;
    assert_equals(&(num_values as usize), &packed_points.len());
    let mut iterator = packed_points.into_iter();
    for expected_value in &values {
        assert_array_equals(expected_value, &java_next(&mut iterator));
    }
    expect_throws_panic("java.util.NoSuchElementException", || {
        java_next(&mut iterator);
    });
    Assert::assert_false(iterator.len() > 0);
    Ok(())
}

/// `it.getClass()`, as the one fact the test compares: whether it is
/// `DocIdSetIterator.RangeDocIdSetIterator`, the class of
/// `DocIdSetIterator.all(int)` (`DocIdSetIterator.java:36-58`), which Rucene
/// splits into [`AllDocIdSetIterator`] and [`RangeDocIdSetIterator`].
///
/// [`AllDocIdSetIterator`]: rucene::search::doc_id_set_iterator::AllDocIdSetIterator
/// [`RangeDocIdSetIterator`]: rucene::search::doc_id_set_iterator::RangeDocIdSetIterator
fn range_class(it: &dyn DocIdSetIterator) -> bool {
    it.disi_as_any().is_some_and(|a| {
        a.is::<doc_id_set_iterator::AllDocIdSetIterator>()
            || a.is::<doc_id_set_iterator::RangeDocIdSetIterator>()
    })
}

/// Port of `TestPointQueries.testRangeOptimizesIfAllPointsMatch`.
fn test_range_optimizes_if_all_points_match() -> Result<()> {
    let num_dims = TestUtil::next_int(&*random(), 1, 3);
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut value = vec![0i32; num_dims as usize];
    for i in 0..num_dims as usize {
        value[i] = TestUtil::next_int(&*random(), 1, 10);
    }
    w.add_document(&doc_with(Box::new(IntPoint::new("point", &value)?)))?;
    let reader = w.get_reader()?;
    let mut searcher = plain_searcher(&reader)?;
    searcher.set_query_cache(None);
    let mut lower_bound = vec![0i32; num_dims as usize];
    let mut upper_bound = vec![0i32; num_dims as usize];
    for i in 0..num_dims as usize {
        lower_bound[i] = value[i] - random().next_int_bound(1);
        upper_bound[i] = value[i] + random().next_int_bound(1);
    }
    let query = IntPoint::new_range_query("point", &lower_bound, &upper_bound)?;
    let weight = searcher.create_weight(Arc::clone(&query), ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    let mut scorer = weight
        .scorer(&Arc::clone(searcher.get_index_reader()).leaves()?[0])?
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
    assert_equals(
        &range_class(&doc_id_set_iterator::all(1)?),
        &range_class(scorer.iterator()),
    );

    // When not all documents in the query have a value, the optimization is not applicable
    drop(scorer);
    drop(weight);
    drop(searcher);
    reader.close()?;
    w.add_document(&Document::new())?;
    w.force_merge(1)?;
    let reader = w.get_reader()?;
    let mut searcher = plain_searcher(&reader)?;
    searcher.set_query_cache(None);
    let weight = searcher.create_weight(Arc::clone(&query), ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    let mut scorer = weight
        .scorer(&Arc::clone(searcher.get_index_reader()).leaves()?[0])?
        .unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
    Assert::assert_false(
        range_class(&doc_id_set_iterator::all(1)?) == range_class(scorer.iterator()),
    );

    drop(scorer);
    drop(weight);
    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testPointRangeWeightCount`.
fn test_point_range_weight_count() -> Result<()> {
    // the optimization for Weight#count kicks in only when the number of dimensions is 1
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;

    let num_points = random().next_int_range(1, 10);
    let mut points = vec![0i32; num_points as usize];

    let num_queries = random().next_int_range(1, 10);
    let mut lower_bound = vec![0i32; num_queries as usize];
    let mut upper_bound = vec![0i32; num_queries as usize];
    let mut expected_count = vec![0i32; num_queries as usize];

    for i in 0..num_queries as usize {
        // generate random queries
        lower_bound[i] = random().next_int_range(1, 10);
        // allow malformed ranges where upperBound could be less than lowerBound
        upper_bound[i] = random().next_int_range(1, 10);
    }

    for i in 0..num_points as usize {
        // generate random 1D points
        points[i] = random().next_int_range(1, 10);
        if random().next_boolean() {
            // the doc may have at-most 1 point
            w.add_document(&doc_with(Box::new(IntPoint::new("point", &[points[i]])?)))?;
            for j in 0..num_queries as usize {
                // calculate the number of points that lie within the query range
                if lower_bound[j] <= points[i] && points[i] <= upper_bound[j] {
                    expected_count[j] += 1;
                }
            }
        }
    }
    w.commit()?;
    w.force_merge(1)?;

    let reader = w.get_reader()?;
    let searcher = plain_searcher(&reader)?;
    if !searcher.get_leaf_contexts().is_empty() {
        // we need at least 1 leaf in the segment
        for i in 0..num_queries as usize {
            let query = IntPoint::new_range_query("point", &[lower_bound[i]], &[upper_bound[i]])?;
            let weight = searcher.create_weight(query, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
            assert_equals(
                &expected_count[i],
                &weight.count(&searcher.get_leaf_contexts()[0])?,
            );
        }
    }

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// `assertEquals(q1, q2); assertEquals(q1.hashCode(), q2.hashCode())`.
fn assert_equal_and_hash(q1: &Arc<dyn Query>, q2: &Arc<dyn Query>) {
    assert_query_equals(q1, q2);
    assert_equals(&q1.query_hash(), &q2.query_hash());
}

/// Port of `TestPointQueries.testPointRangeEquals`.
fn test_point_range_equals() -> Result<()> {
    let q1 = IntPoint::new_range_query("a", &[0], &[1000])?;
    let q2 = IntPoint::new_range_query("a", &[0], &[1000])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*IntPoint::new_range_query("a", &[1], &[1000])?));
    Assert::assert_false(q1.query_eq(&*IntPoint::new_range_query("b", &[0], &[1000])?));

    let q1 = LongPoint::new_range_query("a", &[0], &[1000])?;
    let q2 = LongPoint::new_range_query("a", &[0], &[1000])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*LongPoint::new_range_query("a", &[1], &[1000])?));

    let q1 = FloatPoint::new_range_query("a", &[0.0], &[1000.0])?;
    let q2 = FloatPoint::new_range_query("a", &[0.0], &[1000.0])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*FloatPoint::new_range_query("a", &[1.0], &[1000.0])?));

    let q1 = DoublePoint::new_range_query("a", &[0.0], &[1000.0])?;
    let q2 = DoublePoint::new_range_query("a", &[0.0], &[1000.0])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*DoublePoint::new_range_query("a", &[1.0], &[1000.0])?));

    let zeros = vec![0u8; 5];
    let ones = vec![0xffu8; 5];
    let q1 = BinaryPoint::new_range_query(
        "a",
        &[BytesRef::new(zeros.clone())],
        &[BytesRef::new(ones.clone())],
    )?;
    let q2 = BinaryPoint::new_range_query(
        "a",
        &[BytesRef::new(zeros.clone())],
        &[BytesRef::new(ones.clone())],
    )?;
    assert_equal_and_hash(&q1, &q2);
    let mut other = ones.clone();
    other[2] = 5;
    Assert::assert_false(q1.query_eq(&*BinaryPoint::new_range_query(
        "a",
        &[BytesRef::new(zeros)],
        &[BytesRef::new(other)],
    )?));
    Ok(())
}

/// `(PointRangeQuery) q`, after `assertTrue(q instanceof PointRangeQuery)`.
fn as_point_range(q: &Arc<dyn Query>) -> &PointRangeQuery {
    q.as_any()
        .downcast_ref::<PointRangeQuery>()
        .unwrap_or_else(|| Assert::fail("expected a PointRangeQuery"))
}

/// The `instanceof PointRangeQuery` checks and the lower and upper point
/// comparisons of `testPointExactEquals`.
fn assert_same_points(q1: &Arc<dyn Query>, q2: &Arc<dyn Query>) {
    Assert::assert_true(q1.as_any().is::<PointRangeQuery>() && q2.as_any().is::<PointRangeQuery>());
    let pq1 = as_point_range(q1);
    let pq2 = as_point_range(q2);

    Assert::assert_true(pq1.get_lower_point() == pq2.get_lower_point());
    Assert::assert_true(pq1.get_upper_point() == pq2.get_upper_point());
}

/// Port of `TestPointQueries.testPointExactEquals`.
fn test_point_exact_equals() -> Result<()> {
    let q1 = IntPoint::new_exact_query("a", 1000)?;
    let q2 = IntPoint::new_exact_query("a", 1000)?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*IntPoint::new_exact_query("a", 1)?));
    Assert::assert_false(q1.query_eq(&*IntPoint::new_exact_query("b", 1000)?));
    assert_same_points(&q1, &q2);

    let q1 = LongPoint::new_exact_query("a", 1000)?;
    let q2 = LongPoint::new_exact_query("a", 1000)?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*LongPoint::new_exact_query("a", 1)?));
    assert_same_points(&q1, &q2);

    let q1 = FloatPoint::new_exact_query("a", 1000.0)?;
    let q2 = FloatPoint::new_exact_query("a", 1000.0)?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*FloatPoint::new_exact_query("a", 1.0)?));
    assert_same_points(&q1, &q2);

    let q1 = DoublePoint::new_exact_query("a", 1000.0)?;
    let q2 = DoublePoint::new_exact_query("a", 1000.0)?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*DoublePoint::new_exact_query("a", 1.0)?));
    assert_same_points(&q1, &q2);

    let ones = vec![0xffu8; 5];
    let q1 = BinaryPoint::new_exact_query("a", &ones)?;
    let q2 = BinaryPoint::new_exact_query("a", &ones)?;
    assert_equal_and_hash(&q1, &q2);
    let mut other = ones.clone();
    other[2] = 5;
    Assert::assert_false(q1.query_eq(&*BinaryPoint::new_exact_query("a", &other)?));
    assert_same_points(&q1, &q2);
    Ok(())
}

/// Port of `TestPointQueries.testPointInSetEquals`.
fn test_point_in_set_equals() -> Result<()> {
    let q1 = IntPoint::new_set_query("a", &[0, 1000, 17])?;
    let q2 = IntPoint::new_set_query("a", &[17, 0, 1000])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*IntPoint::new_set_query("a", &[1, 17, 1000])?));
    Assert::assert_false(q1.query_eq(&*IntPoint::new_set_query("b", &[0, 1000, 17])?));

    let q1 = LongPoint::new_set_query("a", &[0, 1000, 17])?;
    let q2 = LongPoint::new_set_query("a", &[17, 0, 1000])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*LongPoint::new_set_query("a", &[1, 17, 1000])?));

    let q1 = FloatPoint::new_set_query("a", &[0.0, 1000.0, 17.0])?;
    let q2 = FloatPoint::new_set_query("a", &[17.0, 0.0, 1000.0])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*FloatPoint::new_set_query("a", &[1.0, 17.0, 1000.0])?));

    let q1 = DoublePoint::new_set_query("a", &[0.0, 1000.0, 17.0])?;
    let q2 = DoublePoint::new_set_query("a", &[17.0, 0.0, 1000.0])?;
    assert_equal_and_hash(&q1, &q2);
    Assert::assert_false(q1.query_eq(&*DoublePoint::new_set_query("a", &[1.0, 17.0, 1000.0])?));

    let zeros = vec![0u8; 5];
    let ones = vec![0xffu8; 5];
    let q1 = BinaryPoint::new_set_query("a", &[zeros.clone(), ones.clone()])?;
    let q2 = BinaryPoint::new_set_query("a", &[zeros.clone(), ones.clone()])?;
    assert_equal_and_hash(&q1, &q2);
    let mut other = ones.clone();
    other[2] = 5;
    Assert::assert_false(q1.query_eq(&*BinaryPoint::new_set_query("a", &[zeros, other])?));
    Ok(())
}

/// The `toString(int, byte[])` of the anonymous `PointRangeQuery` of
/// `testInvalidPointLength`: `"foo"`.
#[derive(Debug)]
struct FooFormatter;

impl DimensionFormatter for FooFormatter {
    fn format(&self, _dimension: i32, _value: &[u8]) -> String {
        "foo".to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Port of `TestPointQueries.testInvalidPointLength`.
fn test_invalid_point_length() -> Result<()> {
    let e = expect_throws(IAE, || {
        PointRangeQuery::new(
            "field",
            vec![0u8; 4],
            vec![0u8; 8],
            1,
            Arc::new(FooFormatter),
        )
    });
    Assert::assert_equals_str(
        None,
        Some("lowerPoint has length=4 but upperPoint has different length=8"),
        e.message(),
    );
    Ok(())
}

/// `Double.compare(a, b) == 0`.
fn double_compare_eq(a: f64, b: f64) -> bool {
    a.total_cmp(&b) == std::cmp::Ordering::Equal
}

/// `Float.compare(a, b) == 0`.
fn float_compare_eq(a: f32, b: f32) -> bool {
    a.total_cmp(&b) == std::cmp::Ordering::Equal
}

/// Port of `TestPointQueries.testNextUp`.
fn test_next_up() -> Result<()> {
    Assert::assert_true(double_compare_eq(0.0, DoublePoint::next_up(-0.0)));
    Assert::assert_true(double_compare_eq(
        DOUBLE_MIN_VALUE,
        DoublePoint::next_up(0.0),
    ));
    Assert::assert_true(double_compare_eq(
        f64::INFINITY,
        DoublePoint::next_up(f64::MAX),
    ));
    Assert::assert_true(double_compare_eq(
        f64::INFINITY,
        DoublePoint::next_up(f64::INFINITY),
    ));
    Assert::assert_true(double_compare_eq(
        -f64::MAX,
        DoublePoint::next_up(f64::NEG_INFINITY),
    ));

    Assert::assert_true(float_compare_eq(0.0, FloatPoint::next_up(-0.0)));
    Assert::assert_true(float_compare_eq(FLOAT_MIN_VALUE, FloatPoint::next_up(0.0)));
    Assert::assert_true(float_compare_eq(
        f32::INFINITY,
        FloatPoint::next_up(f32::MAX),
    ));
    Assert::assert_true(float_compare_eq(
        f32::INFINITY,
        FloatPoint::next_up(f32::INFINITY),
    ));
    Assert::assert_true(float_compare_eq(
        -f32::MAX,
        FloatPoint::next_up(f32::NEG_INFINITY),
    ));
    Ok(())
}

/// Port of `TestPointQueries.testNextDown`.
fn test_next_down() -> Result<()> {
    Assert::assert_true(double_compare_eq(-0.0, DoublePoint::next_down(0.0)));
    Assert::assert_true(double_compare_eq(
        -DOUBLE_MIN_VALUE,
        DoublePoint::next_down(-0.0),
    ));
    Assert::assert_true(double_compare_eq(
        f64::NEG_INFINITY,
        DoublePoint::next_down(-f64::MAX),
    ));
    Assert::assert_true(double_compare_eq(
        f64::NEG_INFINITY,
        DoublePoint::next_down(f64::NEG_INFINITY),
    ));
    Assert::assert_true(double_compare_eq(
        f64::MAX,
        DoublePoint::next_down(f64::INFINITY),
    ));

    Assert::assert_true(float_compare_eq(-0.0, FloatPoint::next_down(0.0)));
    Assert::assert_true(float_compare_eq(
        -FLOAT_MIN_VALUE,
        FloatPoint::next_down(-0.0),
    ));
    Assert::assert_true(float_compare_eq(
        f32::NEG_INFINITY,
        FloatPoint::next_down(-f32::MAX),
    ));
    Assert::assert_true(float_compare_eq(
        f32::NEG_INFINITY,
        FloatPoint::next_down(f32::NEG_INFINITY),
    ));
    Assert::assert_true(float_compare_eq(
        f32::MAX,
        FloatPoint::next_down(f32::INFINITY),
    ));
    Ok(())
}

/// Port of `TestPointQueries.testInversePointRange`.
fn test_inverse_point_range() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let num_dims = TestUtil::next_int(&*random(), 1, 3);
    let num_docs = LuceneTestCase::at_least(10 * BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE); // we need multiple leaves to enable this
                                                                                              // optimization
    for i in 0..num_docs {
        let values = vec![i; num_dims as usize];
        w.add_document(&doc_with(Box::new(IntPoint::new("f", &values)?)))?;
    }
    w.force_merge(1)?;
    let r = directory_reader::open_with_writer(&*w)?;
    w.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&r));
    let mut low = vec![0i32; num_dims as usize];
    let mut high = vec![0i32; num_dims as usize];
    high.fill(num_docs - 2);
    assert_equals(
        &(high[0] - low[0] + 1),
        &searcher.count(IntPoint::new_range_query("f", &low, &high)?)?,
    );
    low.fill(1);
    assert_equals(
        &(high[0] - low[0] + 1),
        &searcher.count(IntPoint::new_range_query("f", &low, &high)?)?,
    );
    high.fill(num_docs - 1);
    assert_equals(
        &(high[0] - low[0] + 1),
        &searcher.count(IntPoint::new_range_query("f", &low, &high)?)?,
    );
    low.fill(BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE + 1);
    assert_equals(
        &(high[0] - low[0] + 1),
        &searcher.count(IntPoint::new_range_query("f", &low, &high)?)?,
    );
    high.fill(num_docs - BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE);
    assert_equals(
        &(high[0] - low[0] + 1),
        &searcher.count(IntPoint::new_range_query("f", &low, &high)?)?,
    );

    drop(searcher);
    r.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testRangeQueryOptimizesRewrites`.
fn test_range_query_optimizes_rewrites() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = RandomIndexWriter::new_with_config(
        &*random(),
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;

    let num_dims = random_int_value(Some(1), Some(MAX_INDEX_DIMENSIONS));
    let mut point = vec![0i32; num_dims as usize];
    let mut lower = vec![0i32; num_dims as usize];
    let mut upper = vec![0i32; num_dims as usize];

    for i in 0..num_dims as usize {
        point[i] = random_int_value(Some(1), Some(10));
        lower[i] = point[i] - random_int_value(Some(0), Some(5));
        upper[i] = point[i] + random_int_value(Some(0), Some(5));
    }

    // Should rewrite to match all docs query if fully contained
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("field", &point)?));
    doc.add(Box::new(SortedNumericDocValuesField::new("field", 1)));
    w.add_document(&doc)?;

    let reader = w.get_reader()?;
    let searcher = plain_searcher(&reader)?;

    let query = IntPoint::new_range_query("field", &lower, &upper)?;
    let rewritten = searcher.rewrite(Arc::clone(&query))?;
    Assert::assert_true_msg(
        // `rewritten.getClass().getName()`: the query's string form stands
        // for the class name, which a `dyn Query` does not carry.
        &format!(
            "Expected MatchAllDocsQuery, but got [{}]",
            query_to_string(&*rewritten)
        ),
        rewritten.as_any().is::<MatchAllDocsQuery>(),
    );
    assert_equals(&1, &searcher.count(Arc::clone(&query))?);

    // Should rewrite to FieldExistsQuery if fully contained but not all docs have values
    w.add_document(&Document::new())?;
    w.force_merge(1)?;

    drop(searcher);
    reader.close()?;
    let reader = w.get_reader()?;
    let searcher = plain_searcher(&reader)?;
    let rewritten = searcher.rewrite(Arc::clone(&query))?;
    Assert::assert_true_msg(
        &format!(
            "Expected FieldExistsQuery, but got [{}]",
            query_to_string(&*rewritten)
        ),
        rewritten.as_any().is::<FieldExistsQuery>(),
    );
    assert_equals(&1, &searcher.count(Arc::clone(&query))?);

    // Should fallback to MatchNoDocsQuery if no docs have values
    for i in 0..num_dims as usize {
        lower[i] = point[i] - 3;
        upper[i] = point[i] - 2;
    }

    let query = IntPoint::new_range_query("field", &lower, &upper)?;
    let rewritten = searcher.rewrite(Arc::clone(&query))?;
    Assert::assert_true_msg(
        &format!(
            "Expected MatchNoDocsQuery, but got [{}]",
            query_to_string(&*rewritten)
        ),
        rewritten.as_any().is::<MatchNoDocsQuery>(),
    );
    assert_equals(&0, &searcher.count(Arc::clone(&query))?);

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// `searcher.createWeight(searcher.rewrite(query), COMPLETE_NO_SCORES,
/// 1f).scorerSupplier(reader.leaves().get(0)) == null`.
fn scorer_supplier_is_null(
    searcher: &IndexSearcher,
    reader: &Arc<dyn DirectoryReader>,
    query: Arc<dyn Query>,
) -> Result<bool> {
    let weight =
        searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
    let leaves = (Arc::clone(reader) as Arc<dyn IndexReader>).leaves()?;
    Ok(weight.scorer_supplier(&leaves[0])?.is_none())
}

/// Port of `TestPointQueries.testRangeQuerySkipsNonMatchingSegments`.
fn test_range_query_skips_non_matching_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("field", &[2])?));
    doc.add(Box::new(IntPoint::new("field2d", &[1, 3])?));
    w.add_document(&doc)?;

    let reader = directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = IntPoint::new_range_query("field", &[0], &[1])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = IntPoint::new_range_query("field", &[3], &[4])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = IntPoint::new_range_query("field2d", &[0, 0], &[2, 2])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = IntPoint::new_range_query("field2d", &[2, 2], &[4, 4])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestPointQueries.testPointInSetQuerySkipsNonMatchingSegments`.
fn test_point_in_set_query_skips_non_matching_segments() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    let mut doc = Document::new();
    doc.add(Box::new(IntPoint::new("field", &[10])?));
    doc.add(Box::new(IntPoint::new("field2d", &[10, 10])?));
    w.add_document(&doc)?;

    let reader = directory_reader::open_with_writer(&*w)?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query = IntPoint::new_set_query("field", &[1, 3, 5])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = IntPoint::new_set_query("field", &[11, 13, 15])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = IntPoint::new_set_query("field", &[5, 10, 15])?;
    Assert::assert_false(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = new_multi_dim_int_set_query("field2d", 2, &[5, 5])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = new_multi_dim_int_set_query("field2d", 2, &[15, 15])?;
    Assert::assert_true(scorer_supplier_is_null(&searcher, &reader, query)?);

    let query = new_multi_dim_int_set_query("field2d", 2, &[10, 10])?;
    Assert::assert_false(scorer_supplier_is_null(&searcher, &reader, query)?);

    drop(searcher);
    reader.close()?;
    w.close()?;
    dir.close()?;
    Ok(())
}

/// The anonymous `PointInSetQuery.Stream` of
/// `testOutOfOrderValuesInPointInSetQuery`: `{2}` then `{1}`, out of order.
struct OutOfOrderStream {
    values: Vec<BytesRef>,
    index: usize,
}

impl BytesRefIterator for OutOfOrderStream {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.index < self.values.len() {
            let v = self.values[self.index].clone();
            self.index += 1;
            Ok(Some(v))
        } else {
            Ok(None)
        }
    }
}

/// Port of `TestPointQueries.testOutOfOrderValuesInPointInSetQuery`.
fn test_out_of_order_values_in_point_in_set_query() -> Result<()> {
    let expected = expect_throws(IAE, || {
        let mut stream = OutOfOrderStream {
            values: vec![
                LuceneTestCase::new_bytes_ref_bytes(&[2]),
                LuceneTestCase::new_bytes_ref_bytes(&[1]), // out of order
            ],
            index: 0,
        };
        PointInSetQuery::new("foo", 1, 1, &mut stream, Arc::new(ArraysToStringFormatter))
    });
    Assert::assert_equals_str(
        None,
        Some("values are out of order: saw [2] before [1]"),
        expected.message(),
    );
    Ok(())
}
