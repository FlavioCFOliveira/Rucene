//! Port of `lucene/core/src/test/org/apache/lucene/search/TestDocValuesQueries.java`.
//!
//! Notes on the translation:
//!
//! * `testSortedNumericDocValuesRangeQueryRewrites(Random)` takes a
//!   parameter, so `LuceneJUnit3MethodProvider` does not select it as a test
//!   (`m.getParameterTypes().length == 0`); it is declared with its
//!   parameter and is not run, as in Lucene.
//! * `getClass().toString()` and `instanceOf(...)` over queries, suppliers
//!   and iterators use the opt-in `as_any` downcasts against a declared list
//!   of the classes the assertions name.

use std::sync::Arc;

use randomizedtesting::jdk::random::Random;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::codecs::lucene90::doc_values::Lucene90DocValuesFormat;
use rucene::codecs::Codec;
use rucene::document::doc_values_queries::SortedNumericDocValuesRangeQuery;
use rucene::document::{
    Document, LongField, LongPoint, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::doc_values::DocValues;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::{IndexReader, IndexableField};
use rucene::search::constant_score_scorer_supplier::{
    ConstantScoreScorerSupplier, SingleIteratorSupplier,
};
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::term_query::TermQuery;
use rucene::search::two_phase_iterator::ScorerIterator;
use rucene::search::{
    query_to_string, BooleanQueryBuilder, BoostQuery, ConstantScoreQuery, DocValuesRangeIterator,
    FieldExistsQuery, MatchAllDocsQuery, MatchNoDocsQuery, Occur, Query, ScoreMode, Sort,
    SortField, SortFieldType, TopDocs,
};
use rucene::util::{BytesRef, NumericUtils};
use rucene_test_framework::hamcrest::{
    greater_than_or_equal_to, less_than_or_equal_to, MatcherAssert,
};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::query_utils::QueryUtils;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::term;
use crate::core_support::{
    assert_equals, assert_equals_delta, assert_not_null, assert_null, class, random, test,
    JavaHashSet,
};

/// `TestDocValuesQueries`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestDocValuesQueries",
        vec![
            test(
                "testDuelPointRangeSortedNumericRangeQuery",
                test_duel_point_range_sorted_numeric_range_query,
            ),
            test(
                "testDuelPointRangeSortedNumericRangeWithSlipperQuery",
                test_duel_point_range_sorted_numeric_range_with_slipper_query,
            ),
            test(
                "testDuelPointRangeMultivaluedSortedNumericRangeQuery",
                test_duel_point_range_multivalued_sorted_numeric_range_query,
            ),
            test(
                "testDuelPointRangeMultivaluedSortedNumericRangeWithSkipperQuery",
                test_duel_point_range_multivalued_sorted_numeric_range_with_skipper_query,
            ),
            test(
                "testDuelPointRangeNumericRangeQuery",
                test_duel_point_range_numeric_range_query,
            ),
            test(
                "testDuelPointRangeNumericRangeWithSkipperQuery",
                test_duel_point_range_numeric_range_with_skipper_query,
            ),
            test(
                "testDuelPointNumericSortedWithSkipperRangeQuery",
                test_duel_point_numeric_sorted_with_skipper_range_query,
            ),
            test(
                "testDuelPointRangeSortedSetRangeQuery",
                test_duel_point_range_sorted_set_range_query,
            ),
            test(
                "testDuelPointRangeSortedSetRangeSkipperQuery",
                test_duel_point_range_sorted_set_range_skipper_query,
            ),
            test(
                "testDuelPointRangeMultivaluedSortedSetRangeQuery",
                test_duel_point_range_multivalued_sorted_set_range_query,
            ),
            test(
                "testDuelPointRangeMultivaluedSortedSetRangeSkipperQuery",
                test_duel_point_range_multivalued_sorted_set_range_skipper_query,
            ),
            test(
                "testDuelPointRangeSortedRangeQuery",
                test_duel_point_range_sorted_range_query,
            ),
            test(
                "testSortedSetRangeQueryUsesTwoPhaseRangeIteratorWithSkipper",
                test_sorted_set_range_query_uses_two_phase_range_iterator_with_skipper,
            ),
            test(
                "testDuelPointRangeSortedRangeSkipperQuery",
                test_duel_point_range_sorted_range_skipper_query,
            ),
            test(
                "testDuelPointSortedSetSortedWithSkipperRangeQuery",
                test_duel_point_sorted_set_sorted_with_skipper_range_query,
            ),
            test("testEquals", test_equals),
            test("testToString", test_to_string),
            test("testMissingField", test_missing_field),
            test("testSlowRangeQueryRewrite", test_slow_range_query_rewrite),
            test("testSortedNumericNPE", test_sorted_numeric_npe),
            test("testSetEquals", test_set_equals),
            test("testDuelSetVsTermsQuery", test_duel_set_vs_terms_query),
            test(
                "testSortedNumericDocValuesRangeQueryCount",
                test_sorted_numeric_doc_values_range_query_count,
            ),
            test(
                "testSortedSetDocValuesRangeQueryCount",
                test_sorted_set_doc_values_range_query_count,
            ),
            // Not a JUnit3 test method: it declares a parameter.
            test("testSortedNumericDocValuesRangeQueryRewrites", || {
                test_sorted_numeric_doc_values_range_query_rewrites(&*random())
            })
            .params(&["java.util.Random"]),
            test(
                "testRewriteWorksWithPointsButNoSkipIndex",
                test_rewrite_works_with_points_but_no_skip_index,
            ),
            test(
                "testPrimarySortDenseSortedDocValuesExactMatch",
                test_primary_sort_dense_sorted_doc_values_exact_match,
            ),
            test(
                "testPrimarySortDenseNumericDocValuesExactMatch",
                test_primary_sort_dense_numeric_doc_values_exact_match,
            ),
        ],
    )
}

/// `getCodec()`: small interval size to test with many intervals.
fn get_codec() -> Arc<dyn Codec> {
    // small interval size to test with many intervals
    TestUtil::always_doc_values_format(Arc::new(Lucene90DocValuesFormat::with_interval(
        random().next_int_range(4, 16),
    )))
}

/// `getCodec(int skipIntervalSize)`.
fn get_codec_with(skip_interval_size: i32) -> Arc<dyn Codec> {
    // small interval size to test with many intervals
    TestUtil::always_doc_values_format(Arc::new(Lucene90DocValuesFormat::with_interval(
        skip_interval_size,
    )))
}

/// `Sort.INDEXORDER`.
fn index_order() -> Sort {
    Sort::new_fields(vec![SortField::FIELD_DOC.clone()]).expect("one sort field")
}

/// `Sort.RELEVANCE`.
fn relevance() -> Sort {
    Sort::new()
}

/// `new Sort(new SortField(field, type, reverse))`.
fn sort_by(field: &str, field_type: SortFieldType, reverse: bool) -> Result<Sort> {
    Sort::new_fields(vec![SortField::new_reverse(
        Some(field.to_string()),
        field_type,
        reverse,
    )?])
}

/// `newBytesRef(byte[])`.
fn nbr(bytes: &[u8]) -> BytesRef {
    LuceneTestCase::new_bytes_ref_bytes(bytes)
}

/// `newBytesRef(String)`.
fn nbr_str(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// `LongPoint.newRangeQuery(field, min, max)`.
fn long_range(field: &str, min: i64, max: i64) -> Result<Arc<dyn Query>> {
    LongPoint::new_range_query(field, &[min], &[max])
}

fn test_duel_point_range_sorted_numeric_range_query() -> Result<()> {
    do_test_duel_point_range_numeric_range_query(true, 1, false)
}

fn test_duel_point_range_sorted_numeric_range_with_slipper_query() -> Result<()> {
    do_test_duel_point_range_numeric_range_query(true, 1, true)
}

fn test_duel_point_range_multivalued_sorted_numeric_range_query() -> Result<()> {
    do_test_duel_point_range_numeric_range_query(true, 3, false)
}

fn test_duel_point_range_multivalued_sorted_numeric_range_with_skipper_query() -> Result<()> {
    do_test_duel_point_range_numeric_range_query(true, 3, true)
}

fn test_duel_point_range_numeric_range_query() -> Result<()> {
    do_test_duel_point_range_numeric_range_query(false, 1, false)
}

fn test_duel_point_range_numeric_range_with_skipper_query() -> Result<()> {
    do_test_duel_point_range_numeric_range_query(false, 1, true)
}

fn test_duel_point_numeric_sorted_with_skipper_range_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = IndexWriterConfig::new();
    config.set_codec(get_codec())?;
    config.set_index_sort(sort_by("dv", SortFieldType::Long, random().next_boolean())?)?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let value = TestUtil::next_long(&*random(), -100, 10000);
        doc.add(Box::new(NumericDocValuesField::indexed_field("dv", value)));
        doc.add(Box::new(LongPoint::new("idx", &[value])?));
        iw.add_document(&doc)?;
    }

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
    iw.close()?;

    for _ in 0..100 {
        let min = if random().next_boolean() {
            i64::MIN
        } else {
            TestUtil::next_long(&*random(), -100, 10000)
        };
        let max = if random().next_boolean() {
            i64::MAX
        } else {
            TestUtil::next_long(&*random(), -100, 10000)
        };
        let q1 = long_range("idx", min, max)?;
        let q2 = NumericDocValuesField::new_slow_range_query("dv", min, max);
        assert_same_matches(&searcher, q1, q2, false)?;
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn do_test_duel_point_range_numeric_range_query(
    sorted_numeric: bool,
    max_values_per_doc: i32,
    skypper: bool,
) -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let dir = LuceneTestCase::new_directory();
        let iw = if sorted_numeric || random().next_boolean() {
            RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?
        } else {
            let config = IndexWriterConfig::new();
            config.set_codec(get_codec())?;
            config.set_index_sort(sort_by("dv", SortFieldType::Long, random().next_boolean())?)?;
            RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?
        };
        let num_docs = LuceneTestCase::at_least(100);
        for _ in 0..num_docs {
            let mut doc = Document::new();
            let num_values = TestUtil::next_int(&*random(), 0, max_values_per_doc);
            for _ in 0..num_values {
                let value = TestUtil::next_long(&*random(), -100, 10000);
                if sorted_numeric {
                    if skypper {
                        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                            "dv", value,
                        )));
                    } else {
                        doc.add(Box::new(SortedNumericDocValuesField::new("dv", value)));
                    }
                } else if skypper {
                    doc.add(Box::new(NumericDocValuesField::indexed_field("dv", value)));
                } else {
                    doc.add(Box::new(NumericDocValuesField::new("dv", value)));
                }
                doc.add(Box::new(LongPoint::new("idx", &[value])?));
            }
            iw.add_document(&doc)?;
        }
        if random().next_boolean() {
            iw.delete_documents_by_query(long_range("idx", 0, 10)?)?;
        }
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
        iw.close()?;

        for _ in 0..100 {
            let min = if random().next_boolean() {
                i64::MIN
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let max = if random().next_boolean() {
                i64::MAX
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let q1 = long_range("idx", min, max)?;
            let q2 = if sorted_numeric {
                SortedNumericDocValuesField::new_slow_range_query("dv", min, max)
            } else {
                NumericDocValuesField::new_slow_range_query("dv", min, max)
            };
            assert_same_matches(&searcher, q1, q2, false)?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

/// `LongPoint.encodeDimension(value, encoded, 0)` into a new
/// `byte[Long.BYTES]`.
fn encode(value: i64) -> [u8; 8] {
    let mut encoded = [0u8; 8];
    LongPoint::encode_dimension(value, &mut encoded, 0);
    encoded
}

fn do_test_duel_point_range_sorted_range_query(
    sorted_set: bool,
    max_values_per_doc: i32,
    skypper: bool,
) -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _ in 0..iters {
        let dir = LuceneTestCase::new_directory();
        let iw = if sorted_set || random().next_boolean() {
            RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?
        } else {
            let config = IndexWriterConfig::new();
            config.set_codec(get_codec())?;
            config.set_index_sort(sort_by(
                "dv",
                SortFieldType::String,
                random().next_boolean(),
            )?)?;
            RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?
        };
        let num_docs = LuceneTestCase::at_least(100);
        for _ in 0..num_docs {
            let mut doc = Document::new();
            let num_values = TestUtil::next_int(&*random(), 0, max_values_per_doc);
            for _ in 0..num_values {
                let value = TestUtil::next_long(&*random(), -100, 10000);
                let encoded = encode(value);
                if sorted_set {
                    if skypper {
                        doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                            "dv",
                            nbr(&encoded),
                        )));
                    } else {
                        doc.add(Box::new(SortedSetDocValuesField::new("dv", nbr(&encoded))));
                    }
                } else if skypper {
                    doc.add(Box::new(SortedDocValuesField::indexed_field(
                        "dv",
                        nbr(&encoded),
                    )));
                } else {
                    doc.add(Box::new(SortedDocValuesField::new("dv", nbr(&encoded))));
                }
                doc.add(Box::new(LongPoint::new("idx", &[value])?));
            }
            iw.add_document(&doc)?;
        }
        if random().next_boolean() {
            iw.delete_documents_by_query(long_range("idx", 0, 10)?)?;
        }
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
        iw.close()?;

        for _ in 0..100 {
            let mut min = if random().next_boolean() {
                i64::MIN
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let mut max = if random().next_boolean() {
                i64::MAX
            } else {
                TestUtil::next_long(&*random(), -100, 10000)
            };
            let encoded_min = encode(min);
            let encoded_max = encode(max);
            let mut include_min = true;
            let mut include_max = true;
            if random().next_boolean() {
                include_min = false;
                min = min.wrapping_add(1);
            }
            if random().next_boolean() {
                include_max = false;
                max = max.wrapping_sub(1);
            }
            let q1 = long_range("idx", min, max)?;
            let lower = if min == i64::MIN && random().next_boolean() {
                None
            } else {
                Some(nbr(&encoded_min))
            };
            let upper = if max == i64::MAX && random().next_boolean() {
                None
            } else {
                Some(nbr(&encoded_max))
            };
            let q2 = if sorted_set {
                SortedSetDocValuesField::new_slow_range_query(
                    "dv",
                    lower,
                    upper,
                    include_min,
                    include_max,
                )
            } else {
                SortedDocValuesField::new_slow_range_query(
                    "dv",
                    lower,
                    upper,
                    include_min,
                    include_max,
                )
            };
            assert_same_matches(&searcher, q1, q2, false)?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_duel_point_range_sorted_set_range_query() -> Result<()> {
    do_test_duel_point_range_sorted_range_query(true, 1, false)
}

fn test_duel_point_range_sorted_set_range_skipper_query() -> Result<()> {
    do_test_duel_point_range_sorted_range_query(true, 1, true)
}

fn test_duel_point_range_multivalued_sorted_set_range_query() -> Result<()> {
    do_test_duel_point_range_sorted_range_query(true, 3, false)
}

fn test_duel_point_range_multivalued_sorted_set_range_skipper_query() -> Result<()> {
    do_test_duel_point_range_sorted_range_query(true, 3, true)
}

fn test_duel_point_range_sorted_range_query() -> Result<()> {
    do_test_duel_point_range_sorted_range_query(false, 1, false)
}

fn test_sorted_set_range_query_uses_two_phase_range_iterator_with_skipper() -> Result<()> {
    assert_sorted_set_range_query_uses_two_phase_range_iterator_with_skipper(false)?;
    assert_sorted_set_range_query_uses_two_phase_range_iterator_with_skipper(true)?;
    Ok(())
}

fn assert_sorted_set_range_query_uses_two_phase_range_iterator_with_skipper(
    multi_valued: bool,
) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let config = IndexWriterConfig::new();
        config.set_codec(get_codec_with(8))?;
        let iw = IndexWriter::new(dir.as_directory(), config)?;
        for i in 0..512 {
            let mut doc = Document::new();
            add_sorted_set_value(&mut doc, i % 100);
            if multi_valued {
                add_sorted_set_value(&mut doc, 100 + (i % 100));
            }
            iw.add_document(&doc)?;
        }
        iw.force_merge(1)?;

        {
            let reader = directory_reader::open_with_writer(&*iw)?;
            let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
            let encoded_min = encode(20);
            let encoded_max = encode(40);
            let query = SortedSetDocValuesField::new_slow_range_query(
                "dv",
                Some(nbr(&encoded_min)),
                Some(nbr(&encoded_max)),
                true,
                true,
            );

            let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
            let leaf = &leaves[0];
            Assert::assert_true(Arc::ptr_eq(&query, &searcher.rewrite(Arc::clone(&query))?)); // assertSame
            assert_null(&leaf.leaf_reader().get_meta_data().sort().cloned());
            assert_not_null(&leaf.leaf_reader().get_doc_values_skipper("dv")?);
            assert_equals(
                &!multi_valued,
                &DocValues::unwrap_singleton_sorted(DocValues::get_sorted_set(
                    &*leaf.leaf_reader(),
                    "dv",
                )?)?
                .is_some(),
            );

            let weight = query.create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
            let mut scorer_supplier = weight.scorer_supplier(leaf)?;

            assert_not_null(&scorer_supplier);
            let scorer_supplier = scorer_supplier.as_mut().expect("asserted non-null");
            let constant_score = scorer_supplier.as_any_mut().and_then(|a| {
                a.downcast_mut::<ConstantScoreScorerSupplier<SingleIteratorSupplier>>()
            });
            Assert::assert_true_msg(
                "scorerSupplier is not a ConstantScoreScorerSupplier",
                constant_score.is_some(),
            );
            let iterator = constant_score
                .expect("asserted instanceof")
                .iterator(i64::MAX)?;
            // The skipper route exposes a two-phase DocValuesRangeIterator whose intoBitSet
            // bulk-evaluates skip blocks, rather than a per-doc plain iterator.
            // `TwoPhaseIterator.unwrap(iterator)`: the two-phase view, or null.
            let iterator_class = format!("{iterator:?}");
            let two_phase = match &iterator {
                ScorerIterator::TwoPhase(tp) => Some(tp),
                ScorerIterator::Simple(_) => None,
            };
            if two_phase.is_none() {
                Assert::fail(&iterator_class);
            }
            let two_phase = two_phase.expect("asserted non-null");
            Assert::assert_true_msg(
                "twoPhase is not a DocValuesRangeIterator",
                two_phase
                    .two_phase_as_any()
                    .is_some_and(|a| a.is::<DocValuesRangeIterator>()),
            );
            drop(iterator);
            drop(weight);
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn add_sorted_set_value(doc: &mut Document, value: i64) {
    let encoded = encode(value);
    doc.add(Box::new(SortedSetDocValuesField::indexed_field(
        "dv",
        nbr(&encoded),
    )));
}

fn test_duel_point_range_sorted_range_skipper_query() -> Result<()> {
    do_test_duel_point_range_sorted_range_query(false, 1, true)
}

fn test_duel_point_sorted_set_sorted_with_skipper_range_query() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let config = IndexWriterConfig::new();
    config.set_codec(get_codec())?;
    config.set_index_sort(sort_by(
        "dv",
        SortFieldType::String,
        random().next_boolean(),
    )?)?;
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    let num_docs = LuceneTestCase::at_least(1000);
    for _ in 0..num_docs {
        let mut doc = Document::new();
        let value = TestUtil::next_long(&*random(), -100, 10000);
        let encoded = encode(value);
        doc.add(Box::new(SortedDocValuesField::indexed_field(
            "dv",
            nbr(&encoded),
        )));
        doc.add(Box::new(LongPoint::new("idx", &[value])?));
        iw.add_document(&doc)?;
    }

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
    iw.close()?;

    for _ in 0..100 {
        let mut min = if random().next_boolean() {
            i64::MIN
        } else {
            TestUtil::next_long(&*random(), -100, 10000)
        };
        let mut max = if random().next_boolean() {
            i64::MAX
        } else {
            TestUtil::next_long(&*random(), -100, 10000)
        };
        let encoded_min = encode(min);
        let encoded_max = encode(max);
        let mut include_min = true;
        let mut include_max = true;
        if random().next_boolean() {
            include_min = false;
            min = min.wrapping_add(1);
        }
        if random().next_boolean() {
            include_max = false;
            max = max.wrapping_sub(1);
        }
        let q1 = long_range("idx", min, max)?;
        let lower = if min == i64::MIN && random().next_boolean() {
            None
        } else {
            Some(nbr(&encoded_min))
        };
        let upper = if max == i64::MAX && random().next_boolean() {
            None
        } else {
            Some(nbr(&encoded_max))
        };
        let q2 = SortedDocValuesField::new_slow_range_query(
            "dv",
            lower,
            upper,
            include_min,
            include_max,
        );
        assert_same_matches(&searcher, q1, q2, false)?;
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn assert_same_matches(
    searcher: &IndexSearcher,
    q1: Arc<dyn Query>,
    q2: Arc<dyn Query>,
    scores: bool,
) -> Result<()> {
    let max_doc = searcher.get_index_reader().max_doc();
    let sort = if scores { relevance() } else { index_order() };
    let td1 = searcher.search_with_sort(q1, max_doc, &sort)?;
    let td2 = searcher.search_with_sort(q2, max_doc, &sort)?;
    assert_equals(&td1.total_hits.value(), &td2.total_hits.value());
    let (sd1, sd2) = (score_docs(&td1), score_docs(&td2));
    for i in 0..sd1.len() {
        assert_equals(&sd1[i].doc, &sd2[i].doc);
        if scores {
            assert_equals_delta(f64::from(sd1[i].score), f64::from(sd2[i].score), 10e-7);
        }
    }
    Ok(())
}

/// `topFieldDocs.scoreDocs`.
fn score_docs(td: &rucene::search::TopFieldDocs) -> Vec<rucene::search::ScoreDoc> {
    crate::core_search::support::score_docs_of(&td.score_docs)
}

fn test_equals() -> Result<()> {
    let q1 = SortedNumericDocValuesField::new_slow_range_query("foo", 3, 5);
    QueryUtils::check_equal(
        &*q1,
        &*SortedNumericDocValuesField::new_slow_range_query("foo", 3, 5),
    );
    QueryUtils::check_unequal(
        &*q1,
        &*SortedNumericDocValuesField::new_slow_range_query("foo", 3, 6),
    );
    QueryUtils::check_unequal(
        &*q1,
        &*SortedNumericDocValuesField::new_slow_range_query("foo", 4, 5),
    );
    QueryUtils::check_unequal(
        &*q1,
        &*SortedNumericDocValuesField::new_slow_range_query("bar", 3, 5),
    );

    let q2 = SortedSetDocValuesField::new_slow_range_query(
        "foo",
        Some(nbr_str("bar")),
        Some(nbr_str("baz")),
        true,
        true,
    );
    QueryUtils::check_equal(
        &*q2,
        &*SortedSetDocValuesField::new_slow_range_query(
            "foo",
            Some(nbr_str("bar")),
            Some(nbr_str("baz")),
            true,
            true,
        ),
    );
    QueryUtils::check_unequal(
        &*q2,
        &*SortedSetDocValuesField::new_slow_range_query(
            "foo",
            Some(nbr_str("baz")),
            Some(nbr_str("baz")),
            true,
            true,
        ),
    );
    QueryUtils::check_unequal(
        &*q2,
        &*SortedSetDocValuesField::new_slow_range_query(
            "foo",
            Some(nbr_str("bar")),
            Some(nbr_str("bar")),
            true,
            true,
        ),
    );
    QueryUtils::check_unequal(
        &*q2,
        &*SortedSetDocValuesField::new_slow_range_query(
            "quux",
            Some(nbr_str("bar")),
            Some(nbr_str("baz")),
            true,
            true,
        ),
    );
    Ok(())
}

fn test_to_string() -> Result<()> {
    let q1 = SortedNumericDocValuesField::new_slow_range_query("foo", 3, 5);
    assert_equals("foo:[3 TO 5]", query_to_string(&*q1).as_str());
    assert_equals("[3 TO 5]", q1.to_query_string("foo").as_str());
    assert_equals("foo:[3 TO 5]", q1.to_query_string("bar").as_str());

    let q2 = SortedSetDocValuesField::new_slow_range_query(
        "foo",
        Some(nbr_str("bar")),
        Some(nbr_str("baz")),
        true,
        true,
    );
    assert_equals(
        "foo:[[62 61 72] TO [62 61 7a]]",
        query_to_string(&*q2).as_str(),
    );
    let q2 = SortedSetDocValuesField::new_slow_range_query(
        "foo",
        Some(nbr_str("bar")),
        Some(nbr_str("baz")),
        false,
        true,
    );
    assert_equals(
        "foo:{[62 61 72] TO [62 61 7a]]",
        query_to_string(&*q2).as_str(),
    );
    let q2 = SortedSetDocValuesField::new_slow_range_query(
        "foo",
        Some(nbr_str("bar")),
        Some(nbr_str("baz")),
        false,
        false,
    );
    assert_equals(
        "foo:{[62 61 72] TO [62 61 7a]}",
        query_to_string(&*q2).as_str(),
    );
    let q2 = SortedSetDocValuesField::new_slow_range_query(
        "foo",
        Some(nbr_str("bar")),
        None,
        true,
        true,
    );
    assert_equals("foo:[[62 61 72] TO *}", query_to_string(&*q2).as_str());
    let q2 = SortedSetDocValuesField::new_slow_range_query(
        "foo",
        None,
        Some(nbr_str("baz")),
        true,
        true,
    );
    assert_equals("foo:{* TO [62 61 7a]]", query_to_string(&*q2).as_str());
    assert_equals("{* TO [62 61 7a]]", q2.to_query_string("foo").as_str());
    assert_equals("foo:{* TO [62 61 7a]]", q2.to_query_string("bar").as_str());
    Ok(())
}

fn test_missing_field() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    iw.add_document(&Document::new())?;
    let reader = iw.get_reader()?;
    iw.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let queries: Vec<Arc<dyn Query>> = vec![
        NumericDocValuesField::new_slow_range_query("foo", 2, 4),
        SortedNumericDocValuesField::new_slow_range_query("foo", 2, 4),
        {
            let (lower, upper) = (nbr_str("abc"), nbr_str("bcd"));
            let (li, ui) = (random().next_boolean(), random().next_boolean());
            SortedDocValuesField::new_slow_range_query("foo", Some(lower), Some(upper), li, ui)
        },
        {
            let (lower, upper) = (nbr_str("abc"), nbr_str("bcd"));
            let (li, ui) = (random().next_boolean(), random().next_boolean());
            SortedSetDocValuesField::new_slow_range_query("foo", Some(lower), Some(upper), li, ui)
        },
    ];
    for query in queries {
        let w = searcher.create_weight(searcher.rewrite(query)?, ScoreMode::COMPLETE, 1.0)?;
        let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
        Assert::assert_true(w.scorer(&leaves[0])?.is_none());
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

fn test_slow_range_query_rewrite() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let reader = iw.get_reader()?;
    iw.close()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    QueryUtils::check_equal(
        &*rewrite_once(
            &NumericDocValuesField::new_slow_range_query("foo", 10, 1),
            &searcher,
        )?,
        &MatchNoDocsQuery::instance(),
    );
    QueryUtils::check_equal(
        &*rewrite_once(
            &NumericDocValuesField::new_slow_range_query("foo", i64::MIN, i64::MAX),
            &searcher,
        )?,
        &FieldExistsQuery::new("foo"),
    );
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `query.rewrite(searcher)`: the query itself when it does not rewrite.
fn rewrite_once(query: &Arc<dyn Query>, searcher: &IndexSearcher) -> Result<Arc<dyn Query>> {
    Ok(query
        .rewrite(searcher)?
        .unwrap_or_else(|| Arc::clone(query)))
}

fn test_sorted_numeric_npe() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let nums: [f64; 8] = [
        -1.7147449030215377E-208,
        -1.6887024655302576E-11,
        1.534911516604164E113,
        0.0,
        2.6947996404505155E-166,
        -2.649722021970773E306,
        6.138239235731689E-198,
        2.3967090122610808E111,
    ];
    for num in nums {
        let mut doc = Document::new();
        doc.add(Box::new(SortedNumericDocValuesField::new(
            "dv",
            NumericUtils::double_to_sortable_long(num),
        )));
        iw.add_document(&doc)?;
    }
    iw.commit()?;
    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    iw.close()?;

    let lo = NumericUtils::double_to_sortable_long(8.701032080293731E-226);
    let hi = NumericUtils::double_to_sortable_long(2.0801416404385346E-41);

    let query = SortedNumericDocValuesField::new_slow_range_query("dv", lo, hi);
    // TODO: assert expected matches
    searcher.search_with_sort(query, searcher.get_index_reader().max_doc(), &index_order())?;

    // swap order, should still work
    let query = SortedNumericDocValuesField::new_slow_range_query("dv", hi, lo);
    // TODO: assert expected matches
    searcher.search_with_sort(query, searcher.get_index_reader().max_doc(), &index_order())?;

    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `assertEquals(expected, actual)` / `assertFalse(a.equals(b))` over
/// `Query.equals`.
fn query_equals(a: &Arc<dyn Query>, b: &Arc<dyn Query>) -> bool {
    a.query_eq(&**b)
}

fn test_set_equals() -> Result<()> {
    crate::core_search::test_boost_query::assert_query_equals(
        &NumericDocValuesField::new_slow_set_query("field", &[17, 42]),
        &NumericDocValuesField::new_slow_set_query("field", &[17, 42]),
    );
    crate::core_search::test_boost_query::assert_query_equals(
        &NumericDocValuesField::new_slow_set_query("field", &[17, 42, 32416190071]),
        &NumericDocValuesField::new_slow_set_query("field", &[17, 32416190071, 42]),
    );
    Assert::assert_false(query_equals(
        &NumericDocValuesField::new_slow_set_query("field", &[42]),
        &NumericDocValuesField::new_slow_set_query("field2", &[42]),
    ));
    Assert::assert_false(query_equals(
        &NumericDocValuesField::new_slow_set_query("field", &[17, 42]),
        &NumericDocValuesField::new_slow_set_query("field", &[17, 32416190071]),
    ));
    Ok(())
}

fn test_duel_set_vs_terms_query() -> Result<()> {
    let iters = LuceneTestCase::at_least(2);
    for _ in 0..iters {
        let mut all_numbers: Vec<i64> = Vec::new();
        let num_numbers =
            TestUtil::next_int(&*random(), 1, 1 << TestUtil::next_int(&*random(), 1, 10));
        for _ in 0..num_numbers {
            all_numbers.push(random().next_long());
        }
        let dir = LuceneTestCase::new_directory();
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let num_docs = LuceneTestCase::at_least(100);
        for _ in 0..num_docs {
            let mut doc = Document::new();
            let number = all_numbers[random().next_int_bound(all_numbers.len() as i32) as usize];
            doc.add(Box::new(StringField::new(
                "text",
                number.to_string(),
                Store::NO,
            )?));
            doc.add(Box::new(NumericDocValuesField::new("long", number)));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "twolongs", number,
            )));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "twolongs",
                number.wrapping_mul(2),
            )));
            iw.add_document(&doc)?;
        }
        if num_numbers > 1 && random().next_boolean() {
            iw.delete_documents(term("text", &all_numbers[0].to_string()))?;
        }
        iw.commit()?;
        let reader = iw.get_reader()?;
        let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
        iw.close()?;

        if reader.num_docs() == 0 {
            // may occasionally happen if all documents got the same term
            drop(searcher);
            reader.close()?;
            dir.close()?;
            continue;
        }

        for _ in 0..100 {
            let boost = random().next_float() * 10.0;
            let num_query_numbers =
                TestUtil::next_int(&*random(), 1, 1 << TestUtil::next_int(&*random(), 1, 8));
            let mut query_numbers: JavaHashSet<i64> = JavaHashSet::new();
            let mut query_numbers_x2: JavaHashSet<i64> = JavaHashSet::new();
            for _ in 0..num_query_numbers {
                let number =
                    all_numbers[random().next_int_bound(all_numbers.len() as i32) as usize];
                query_numbers.add(number);
                query_numbers_x2.add(number.wrapping_mul(2));
            }
            let query_numbers_array: Vec<i64> = query_numbers.iter().copied().collect();
            let query_numbers_x2_array: Vec<i64> = query_numbers_x2.iter().copied().collect();
            let mut bq = BooleanQueryBuilder::new();
            for number in query_numbers.iter() {
                bq.add(
                    Arc::new(TermQuery::new(term("text", &number.to_string()))),
                    Occur::SHOULD,
                )?;
            }
            let q1: Arc<dyn Query> = Arc::new(BoostQuery::new(
                Arc::new(ConstantScoreQuery::new(Arc::new(bq.build()))),
                boost,
            )?);

            let q2: Arc<dyn Query> = Arc::new(BoostQuery::new(
                NumericDocValuesField::new_slow_set_query("long", &query_numbers_array),
                boost,
            )?);
            assert_same_matches(&searcher, Arc::clone(&q1), q2, true)?;

            let q3: Arc<dyn Query> = Arc::new(BoostQuery::new(
                SortedNumericDocValuesField::new_slow_set_query("twolongs", &query_numbers_array),
                boost,
            )?);
            assert_same_matches(&searcher, Arc::clone(&q1), q3, true)?;

            let q4: Arc<dyn Query> = Arc::new(BoostQuery::new(
                SortedNumericDocValuesField::new_slow_set_query(
                    "twolongs",
                    &query_numbers_x2_array,
                ),
                boost,
            )?);
            assert_same_matches(&searcher, Arc::clone(&q1), q4, true)?;
        }

        drop(searcher);
        reader.close()?;
        dir.close()?;
    }
    Ok(())
}

fn test_sorted_numeric_doc_values_range_query_count() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        for i in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "with_index",
                100 + i,
            )));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "without_index",
                100 + i,
            )));
            if i != 55 {
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "sparse",
                    100 + i,
                )));
            }
            iw.add_document(&doc)?;
        }
        iw.commit()?;
        iw.force_merge(1)?;

        {
            let reader = iw.get_reader()?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let snrq = SortedNumericDocValuesField::new_slow_range_query;

            assert_count(&searcher, snrq("with_index", 0, 50), 0)?;
            assert_count(&searcher, snrq("without_index", 0, 50), -1)?;
            assert_count(&searcher, snrq("sparse", 0, 50), 0)?;

            assert_count(&searcher, snrq("with_index", 50, 250), 100)?;
            assert_count(&searcher, snrq("without_index", 50, 250), -1)?;
            assert_count(&searcher, snrq("sparse", 50, 250), -1)?;

            assert_count(&searcher, snrq("with_index", 150, 250), -1)?;
            assert_count(&searcher, snrq("without_index", 150, 250), -1)?;
            assert_count(&searcher, snrq("sparse", 150, 250), -1)?;

            assert_count(&searcher, snrq("with_index", 250, 350), 0)?;
            assert_count(&searcher, snrq("without_index", 250, 350), -1)?;
            assert_count(&searcher, snrq("sparse", 250, 350), 0)?;
            drop(searcher);
            reader.close()?;
        }

        iw.delete_documents_by_query(SortedNumericDocValuesField::new_slow_range_query(
            "with_index",
            102,
            103,
        ))?;
        iw.commit()?;

        {
            let reader = iw.get_reader()?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let snrq = SortedNumericDocValuesField::new_slow_range_query;

            assert_count(&searcher, snrq("with_index", 0, 50), 0)?;
            assert_count(&searcher, snrq("with_index", 50, 250), 98)?;
            assert_count(&searcher, snrq("with_index", 150, 250), -1)?;
            assert_count(&searcher, snrq("with_index", 250, 350), 0)?;
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn assert_count(
    searcher: &IndexSearcher,
    query: Arc<dyn Query>,
    expected_count: i32,
) -> Result<()> {
    let w = searcher.create_weight(query, ScoreMode::COMPLETE, 1.0)?;
    let leaves = Arc::clone(searcher.get_index_reader()).leaves()?;
    assert_equals(&expected_count, &w.count(&leaves[0])?);
    Ok(())
}

fn test_sorted_set_doc_values_range_query_count() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        for i in 0..100 {
            let val = format!("{i:03}");
            let mut doc = Document::new();
            doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                "with_index",
                nbr_str(&val),
            )));
            doc.add(Box::new(SortedSetDocValuesField::new(
                "without_index",
                nbr_str(&val),
            )));
            if i != 55 {
                doc.add(Box::new(SortedSetDocValuesField::indexed_field(
                    "sparse",
                    nbr_str(&val),
                )));
            }
            iw.add_document(&doc)?;
        }
        iw.commit()?;
        iw.force_merge(1)?;

        // `SortedSetDocValuesField.newSlowRangeQuery(field, newBytesRef(lo),
        // newBytesRef(hi), true, true)`.
        let ssrq = |field: &str, lo: &str, hi: &str| {
            let (lo, hi) = (nbr_str(lo), nbr_str(hi));
            SortedSetDocValuesField::new_slow_range_query(field, Some(lo), Some(hi), true, true)
        };

        {
            let reader = iw.get_reader()?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

            // Nonexistent field
            assert_count(&searcher, ssrq("nonexistent", "000", "099"), 0)?;

            // Below all values: range is entirely before "000"
            assert_count(&searcher, ssrq("with_index", "!", "/"), 0)?;
            assert_count(&searcher, ssrq("without_index", "!", "/"), 0)?;
            assert_count(&searcher, ssrq("sparse", "!", "/"), 0)?;

            // Match all values: "000" through "099"
            assert_count(&searcher, ssrq("with_index", "000", "099"), 100)?;
            assert_count(&searcher, ssrq("without_index", "000", "099"), -1)?;
            assert_count(&searcher, ssrq("sparse", "000", "099"), -1)?;

            // Partial match: "050" through "060" = 11 values
            assert_count(&searcher, ssrq("with_index", "050", "060"), -1)?;
            assert_count(&searcher, ssrq("without_index", "050", "060"), -1)?;
            assert_count(&searcher, ssrq("sparse", "050", "060"), -1)?;

            // Partial match with bounds not in the index: "0501" falls between "050" and "051"
            assert_count(&searcher, ssrq("with_index", "0501", "100"), -1)?;
            assert_count(&searcher, ssrq("without_index", "0501", "100"), -1)?;
            assert_count(&searcher, ssrq("sparse", "0501", "100"), -1)?;

            // Non-indexed bounds that cover all values: "//" < "000" and "1000" > "099"
            assert_count(&searcher, ssrq("with_index", "//", "1000"), 100)?;
            assert_count(&searcher, ssrq("without_index", "//", "1000"), -1)?;
            assert_count(&searcher, ssrq("sparse", "//", "1000"), -1)?;

            // Non-indexed bounds that match nothing: "0991" through "0999" are above all values
            assert_count(&searcher, ssrq("with_index", "0991", "0999"), 0)?;
            assert_count(&searcher, ssrq("without_index", "0991", "0999"), 0)?;
            assert_count(&searcher, ssrq("sparse", "0991", "0999"), 0)?;

            // Above all values: range is entirely after "099"
            assert_count(&searcher, ssrq("with_index", "100", "199"), 0)?;
            assert_count(&searcher, ssrq("without_index", "100", "199"), 0)?;
            assert_count(&searcher, ssrq("sparse", "100", "199"), 0)?;
            drop(searcher);
            reader.close()?;
        }

        // Delete docs with values "020" through "030" (11 docs)
        iw.delete_documents_by_query(ssrq("with_index", "020", "030"))?;
        iw.commit()?;

        {
            let reader = iw.get_reader()?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;

            // Below all values still matches nothing
            assert_count(&searcher, ssrq("with_index", "!", "/"), 0)?;
            // All values minus 11 deleted = 89
            assert_count(&searcher, ssrq("with_index", "000", "099"), 89)?;
            // Partial match still can't be counted cheaply
            assert_count(&searcher, ssrq("with_index", "050", "060"), -1)?;
            // Above all values still matches nothing
            assert_count(&searcher, ssrq("with_index", "100", "199"), 0)?;
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

/// The Java class name of the queries these assertions name
/// (`getClass().getName()`), from a declared list.
fn query_class_name(q: &Arc<dyn Query>) -> &'static str {
    let any = q.as_any();
    if any.is::<MatchNoDocsQuery>() {
        "org.apache.lucene.search.MatchNoDocsQuery"
    } else if any.is::<MatchAllDocsQuery>() {
        "org.apache.lucene.search.MatchAllDocsQuery"
    } else if any.is::<SortedNumericDocValuesRangeQuery>() {
        "org.apache.lucene.document.SortedNumericDocValuesRangeQuery"
    } else if any.is::<FieldExistsQuery>() {
        "org.apache.lucene.search.FieldExistsQuery"
    } else {
        "<a class outside the declared list>"
    }
}

/// `assertThat(query, instanceOf(class))`.
fn assert_query_instance_of(query: &Arc<dyn Query>, class_name: &str) {
    if query_class_name(query) != class_name {
        Assert::fail(&format!(
            "\nExpected: an instance of {class_name}\n     but: <{}> is a {}",
            query_to_string(&**query),
            query_class_name(query)
        ));
    }
}

/// `assertThat(query.getClass().toString(), containsString(part))`.
fn assert_class_to_string_contains(query: &Arc<dyn Query>, part: &str) {
    let class_to_string = format!("class {}", query_class_name(query));
    crate::core_support::hamcrest::assert_contains_string(Some(&class_to_string), part);
}

fn test_sorted_numeric_doc_values_range_query_rewrites(_random: &dyn Random) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        for i in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                "with_index",
                100 + i,
            )));
            doc.add(Box::new(SortedNumericDocValuesField::new(
                "without_index",
                100 + i,
            )));
            if i % 17 == 0 {
                iw.commit()?;
            }
            if i != 55 {
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "sparse",
                    100 + i,
                )));
            }
            if i == 74 {
                doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
                    "super_sparse",
                    174,
                )));
            }
            iw.add_document(&doc)?;
        }
        iw.commit()?;

        {
            let reader = iw.get_reader()?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            let snrq = SortedNumericDocValuesField::new_slow_range_query;
            assert_query_instance_of(
                &searcher.rewrite(snrq("with_index", 0, 50))?,
                "org.apache.lucene.search.MatchNoDocsQuery",
            );
            assert_query_instance_of(
                &searcher.rewrite(snrq("with_index", 0, 250))?,
                "org.apache.lucene.search.MatchAllDocsQuery",
            );
            assert_query_instance_of(
                &searcher.rewrite(snrq("sparse", 0, 50))?,
                "org.apache.lucene.search.MatchNoDocsQuery",
            );
            assert_query_instance_of(
                &searcher.rewrite(snrq("super_sparse", 0, 50))?,
                "org.apache.lucene.search.MatchNoDocsQuery",
            );
            assert_query_instance_of(
                &searcher.rewrite(snrq("super_sparse", 250, 350))?,
                "org.apache.lucene.search.MatchNoDocsQuery",
            );
            assert_class_to_string_contains(
                &searcher.rewrite(snrq("super_sparse", 174, 174))?,
                "SortedNumericDocValuesRangeQuery",
            );
            assert_class_to_string_contains(
                &searcher.rewrite(snrq("with_index", 0, 150))?,
                "SortedNumericDocValuesRangeQuery",
            );
            assert_class_to_string_contains(
                &searcher.rewrite(snrq("with_index", 150, 250))?,
                "SortedNumericDocValuesRangeQuery",
            );
            assert_class_to_string_contains(
                &searcher.rewrite(snrq("with_index", 120, 150))?,
                "SortedNumericDocValuesRangeQuery",
            );
            assert_class_to_string_contains(
                &searcher.rewrite(snrq("sparse", 0, 250))?,
                "SortedNumericDocValuesRangeQuery",
            );
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

fn test_rewrite_works_with_points_but_no_skip_index() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    {
        let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        for i in 0..100 {
            let mut doc = Document::new();
            doc.add(Box::new(LongField::new("field", 100 + i, Store::NO)));
            iw.add_document(&doc)?;
        }
        iw.commit()?;
        {
            let reader = iw.get_reader()?;
            let searcher = IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>)?;
            // Query range [0, 50] is entirely below field range [100, 199]
            let query = SortedNumericDocValuesField::new_slow_range_query("field", 0, 50);
            let rewritten = searcher.rewrite(query)?;
            assert_query_instance_of(&rewritten, "org.apache.lucene.search.MatchNoDocsQuery");

            // Query range [0, 250] covers entire field range [100, 199]
            // and all docs have a value
            let query = SortedNumericDocValuesField::new_slow_range_query("field", 0, 250);
            let rewritten = searcher.rewrite(query)?;
            assert_query_instance_of(&rewritten, "org.apache.lucene.search.MatchAllDocsQuery");
            drop(searcher);
            reader.close()?;
        }
        iw.close()?;
    }
    dir.close()?;
    Ok(())
}

/// The `IntFunction<IndexableField>` of the exact-match tests: the field for
/// block `i`, shared so that it can be added to two documents, as Java does.
type FieldFactory = fn(i32) -> Arc<dyn IndexableField>;

fn test_primary_sort_dense_sorted_doc_values_exact_match() -> Result<()> {
    do_test_primary_sort_dense_exact_match(
        SortFieldType::String,
        |i| {
            Arc::new(SortedDocValuesField::indexed_field(
                "dv",
                nbr_str(&i.to_string()),
            ))
        },
        |i| {
            let (lo, hi) = (nbr_str(&i.to_string()), nbr_str(&i.to_string()));
            SortedDocValuesField::new_slow_range_query("dv", Some(lo), Some(hi), true, true)
        },
    )
}

fn test_primary_sort_dense_numeric_doc_values_exact_match() -> Result<()> {
    do_test_primary_sort_dense_exact_match(
        SortFieldType::Long,
        |i| Arc::new(NumericDocValuesField::indexed_field("dv", i64::from(i))),
        |i| NumericDocValuesField::new_slow_range_query("dv", i64::from(i), i64::from(i)),
    )
}

/// `public void doTestPrimarySortDenseExactMatch(SortField.Type type,
/// IntFunction<IndexableField> fields, IntFunction<Query> queries)` (a
/// public method that is not a test: it declares parameters).
fn do_test_primary_sort_dense_exact_match(
    field_type: SortFieldType,
    fields: FieldFactory,
    queries: fn(i32) -> Arc<dyn Query>,
) -> Result<()> {
    let deletes = random().next_boolean();
    let dir = LuceneTestCase::new_directory();
    let skip_interval_size = random().next_int_range(4, 4096);
    let config = IndexWriterConfig::new();
    config.set_codec(get_codec_with(skip_interval_size))?;
    config.set_index_sort(sort_by("dv", field_type, random().next_boolean())?)?;
    let num_blocks = random().next_int_range(4, 16);
    let mut sizes = vec![0i32; num_blocks as usize];
    for i in 0..num_blocks as usize {
        sizes[i] = random().next_int_range(1, 250);
    }
    let mut total_sizes = vec![0i32; num_blocks as usize];
    let iw = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), config)?;
    for i in 0..num_blocks as usize {
        total_sizes[i] = sizes[i];
        for _ in 0..sizes[i] {
            let mut doc = Document::new();
            let field = fields(i as i32);
            doc.add_shared(Arc::clone(&field));
            iw.add_document(&doc)?;
            if deletes && random().next_int_bound(10) == 0 {
                let mut doc = Document::new();
                doc.add_shared(field);
                doc.add(Box::new(StringField::new(
                    "id",
                    "to_delete".to_string(),
                    Store::NO,
                )?));
                iw.add_document(&doc)?;
                total_sizes[i] += 1;
            }
        }
    }
    iw.commit()?;
    iw.force_merge(1)?;

    if deletes {
        iw.delete_documents_by_query(Arc::new(TermQuery::new(term("id", "to_delete"))))?;
    }

    let reader = iw.get_reader()?;
    let searcher = LuceneTestCase::new_searcher_wrap(Arc::clone(&reader), false);
    iw.close()?;

    for i in 0..num_blocks as usize {
        let q = queries(i as i32);
        assert_equals(&sizes[i], &searcher.count(Arc::clone(&q))?);
        let top_docs: TopDocs = searcher.search(Arc::clone(&q), 1000)?;
        assert_equals(&i64::from(sizes[i]), &top_docs.total_hits.value());
        // check cost
        let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
        assert_equals(&1, &leaves.len());
        let ctx = &leaves[0];
        let rewritten = searcher.rewrite(q)?;
        let weight = rewritten.create_weight(&searcher, ScoreMode::COMPLETE_NO_SCORES, 1.0)?;
        let supplier = weight.scorer_supplier(ctx)?.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::throw_without_message(
                "java.lang.NullPointerException",
            )
        });
        MatcherAssert::assert_that(
            &supplier.cost(),
            &greater_than_or_equal_to(i64::from(sizes[i])),
        );
        MatcherAssert::assert_that(
            &supplier.cost(),
            &less_than_or_equal_to(i64::from(total_sizes[i]) + 2 * i64::from(skip_interval_size)),
        );
    }
    drop(searcher);
    reader.close()?;
    dir.close()?;
    Ok(())
}
