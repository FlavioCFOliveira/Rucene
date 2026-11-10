//! Port of `lucene/core/src/test/org/apache/lucene/search/comparators/TestNumericComparator.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    Document, DoublePoint, FloatPoint, IntPoint, LongPoint, NumericDocValuesField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::reader_context::LeafReaderContext;
use rucene::index::IndexReader;
use rucene::search::comparators::{
    DoubleComparator, FloatComparator, IntComparator, LongComparator,
};
use rucene::search::field_comparator::{SharedFieldComparator, SlotComparator, SortValue};
use rucene::search::pruning::Pruning;
use rucene::search::scorable::Scorable;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::total_hits::TotalHitsRelation;
use rucene::search::{MatchAllDocsQuery, NO_MORE_DOCS};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_search::support::close_dir;
use crate::core_support::{assert_equals, class, random, test};

/// `TestNumericComparator`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.comparators.TestNumericComparator",
        vec![
            test(
                "testEmptyCompetitiveIteratorOptimization",
                test_empty_competitive_iterator_optimization,
            ),
            test(
                "testEmptyCompetitiveIteratorOptimizationWithMissingValue",
                test_empty_competitive_iterator_optimization_with_missing_value,
            ),
            test(
                "testEmptyCompetitiveIteratorOptimizationAndHitsThresholdReached",
                test_empty_competitive_iterator_optimization_and_hits_threshold_reached,
            ),
        ],
    )
}

/// `new IndexWriterConfig().setMergePolicy(newLogMergePolicy())`.
fn iwc() -> Result<IndexWriterConfig> {
    let mut c = IndexWriterConfig::new();
    c.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    Ok(c)
}

fn test_empty_competitive_iterator_optimization() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(dir.as_directory(), iwc()?)?;
        let body = (|| -> Result<()> {
            for i in 0..num_docs {
                let mut doc = Document::new();
                let l = i64::from(i);

                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "long_field_1",
                    l,
                )));
                doc.add(Box::new(LongPoint::new("long_field_2", &[l])?));
                doc.add(Box::new(NumericDocValuesField::new("long_field_2", l)));

                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "int_field_1",
                    l,
                )));
                doc.add(Box::new(IntPoint::new("int_field_2", &[i])?));
                doc.add(Box::new(NumericDocValuesField::new("int_field_2", l)));

                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "float_field_1",
                    l,
                )));
                doc.add(Box::new(FloatPoint::new("float_field_2", &[i as f32])?));
                doc.add(Box::new(NumericDocValuesField::new("float_field_2", l)));

                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "double_field_1",
                    l,
                )));
                doc.add(Box::new(DoublePoint::new(
                    "double_field_2",
                    &[f64::from(i)],
                )?));
                doc.add(Box::new(NumericDocValuesField::new("double_field_2", l)));

                writer.add_document(&doc)?;
            }
            writer.force_merge(1)?;
            let reader = directory_reader::open_with_writer(&*writer)?;
            let checked = (|| -> Result<()> {
                let leaves = (Arc::clone(&reader) as Arc<dyn IndexReader>).leaves()?;
                assert_equals(&1, &leaves.len());
                let leaf_context = &leaves[0];

                assert_long_field("long_field_1", false, -1, leaf_context)?;
                assert_long_field("long_field_1", true, num_docs + 1, leaf_context)?;
                assert_long_field("long_field_2", false, -1, leaf_context)?;
                assert_long_field("long_field_2", true, num_docs + 1, leaf_context)?;

                assert_int_field("int_field_1", false, -1, leaf_context)?;
                assert_int_field("int_field_1", true, num_docs + 1, leaf_context)?;
                assert_int_field("int_field_2", false, -1, leaf_context)?;
                assert_int_field("int_field_2", true, num_docs + 1, leaf_context)?;

                assert_float_field("float_field_1", false, -1, leaf_context)?;
                assert_float_field("float_field_1", true, num_docs + 1, leaf_context)?;
                assert_float_field("float_field_2", false, -1, leaf_context)?;
                assert_float_field("float_field_2", true, num_docs + 1, leaf_context)?;

                assert_double_field("double_field_1", false, -1, leaf_context)?;
                assert_double_field("double_field_1", true, num_docs + 1, leaf_context)?;
                assert_double_field("double_field_2", false, -1, leaf_context)?;
                assert_double_field("double_field_2", true, num_docs + 1, leaf_context)?;
                Ok(())
            })();
            checked.and(reader.close())
        })();
        body.and(writer.close())
    })();
    result.and(close_dir(Some(dir)))
}

/// `((FieldDoc) topDocs.scoreDocs[i]).fields[0]`.
fn field0(docs: &rucene::search::TopFieldDocs, i: usize) -> SortValue {
    docs.score_docs[i]
        .fields
        .as_ref()
        .and_then(|f| f.first().cloned())
        .unwrap_or(SortValue::Null)
}

fn test_empty_competitive_iterator_optimization_with_missing_value() -> Result<()> {
    let num_docs = LuceneTestCase::at_least(1000);
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(dir.as_directory(), iwc()?)?;
        let body = (|| -> Result<()> {
            for i in 0..num_docs {
                let mut doc = Document::new();
                let l = i64::from(i);

                doc.add(Box::new(NumericDocValuesField::indexed_field(
                    "long_field_1",
                    l,
                )));
                doc.add(Box::new(LongPoint::new("long_field_2", &[l])?));
                doc.add(Box::new(NumericDocValuesField::new("long_field_2", l)));

                writer.add_document(&doc)?;
                if i % 100 == 0 {
                    writer.flush()?;
                }
            }

            let mut doc = Document::new();
            doc.add(Box::new(NumericDocValuesField::new(
                "another_field",
                i64::from(num_docs),
            )));
            writer.add_document(&doc)?;
            writer.flush()?;

            let reader = directory_reader::open_with_writer(&*writer)?;
            let checked = (|| -> Result<()> {
                let mut index_searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
                index_searcher.set_query_cache(None);
                for field in ["long_field_1", "long_field_2"] {
                    let sort_field = SortField::new_with_missing(
                        Some(field.to_string()),
                        SortFieldType::Long,
                        false,
                        Some(MissingValue::Long(i64::MIN)),
                    )?;
                    let top_docs = index_searcher.search_with_sort(
                        Arc::new(MatchAllDocsQuery::instance()),
                        3,
                        &Sort::new_fields(vec![sort_field])?,
                    )?;
                    assert_equals(&num_docs, &top_docs.score_docs[0].score_doc.doc);
                    assert_equals(&SortValue::Long(i64::MIN), &field0(&top_docs, 0));
                    assert_equals(&0, &top_docs.score_docs[1].score_doc.doc);
                    assert_equals(&SortValue::Long(0), &field0(&top_docs, 1));
                    assert_equals(&1, &top_docs.score_docs[2].score_doc.doc);
                    assert_equals(&SortValue::Long(1), &field0(&top_docs, 2));
                }
                Ok(())
            })();
            checked.and(reader.close())
        })();
        body.and(writer.close())
    })();
    result.and(close_dir(Some(dir)))
}

fn test_empty_competitive_iterator_optimization_and_hits_threshold_reached() -> Result<()> {
    let num_docs = TestUtil::next_int(&*random(), 128, 512); // Below IndexSearcher.DEFAULT_HITS_THRESHOLD
    let dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = IndexWriter::new(dir.as_directory(), iwc()?)?;
        let body = (|| -> Result<()> {
            for i in 0..num_docs {
                let mut doc = Document::new();
                let l = i64::from(i);

                doc.add(Box::new(NumericDocValuesField::indexed_field("field_1", l)));
                doc.add(Box::new(LongPoint::new("field_2", &[l])?));
                doc.add(Box::new(NumericDocValuesField::new("field_2", l)));

                writer.add_document(&doc)?;
                if i % 100 == 0 {
                    writer.flush()?;
                }
            }

            let reader = directory_reader::open_with_writer(&*writer)?;
            let checked = (|| -> Result<()> {
                let mut index_searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
                index_searcher.set_query_cache(None);
                for field in ["field_1", "field_2"] {
                    let sort_field = SortField::new_reverse(
                        Some(field.to_string()),
                        SortFieldType::Long,
                        false,
                    )?;
                    let top_docs = index_searcher.search_with_sort(
                        Arc::new(MatchAllDocsQuery::instance()),
                        3,
                        &Sort::new_fields(vec![sort_field])?,
                    )?;
                    assert_equals(
                        &TotalHitsRelation::EQUAL_TO,
                        &top_docs.total_hits.relation(),
                    );
                    assert_equals(&i64::from(num_docs), &top_docs.total_hits.value());
                    assert_equals(&0, &top_docs.score_docs[0].score_doc.doc);
                    assert_equals(&SortValue::Long(0), &field0(&top_docs, 0));
                    assert_equals(&1, &top_docs.score_docs[1].score_doc.doc);
                    assert_equals(&SortValue::Long(1), &field0(&top_docs, 1));
                    assert_equals(&2, &top_docs.score_docs[2].score_doc.doc);
                    assert_equals(&SortValue::Long(2), &field0(&top_docs, 2));
                }
                Ok(())
            })();
            checked.and(reader.close())
        })();
        body.and(writer.close())
    })();
    result.and(close_dir(Some(dir)))
}

/// `leafComparator.setScorer(null)`.
///
/// **Divergence from Lucene 10.5.0.** `setScorer` takes a `&mut dyn
/// Scorable`, which cannot be null; the numeric leaf comparator only tests
/// `scorer instanceof Scorer`, which is false both for `null` and for this
/// scorable, so the comparator takes the same branch (`maxDoc` as the
/// iterator cost).
struct NullScorable;

impl Scorable for NullScorable {
    fn score(&mut self) -> Result<f32> {
        Ok(0.0)
    }
}

/// The body each `assert<Type>Field` shares: the comparator cast to its
/// class `C`, its protected fields set through `set`.
fn assert_field<C: SlotComparator + 'static>(
    field_name: &str,
    field_type: SortFieldType,
    reverse: bool,
    leaf_context: &LeafReaderContext,
    set: impl FnOnce(&mut C),
) -> Result<()> {
    let mut comparator1 =
        SortField::new_reverse(Some(field_name.to_string()), field_type, reverse)?
            .get_comparator(1, Pruning::GREATER_THAN_OR_EQUAL_TO)?;
    comparator1
        .as_any()
        .downcast_ref::<SharedFieldComparator<C>>()
        .expect("ClassCastException")
        .with_comparator(set);
    let mut leaf_comparator = comparator1.get_leaf_comparator(leaf_context)?;
    leaf_comparator.set_scorer(&mut NullScorable)?;
    let mut competitive = leaf_comparator
        .competitive_iterator()?
        .expect("competitive iterator");
    assert_equals(&NO_MORE_DOCS, &competitive.next_doc()?);
    Ok(())
}

fn assert_long_field(
    field_name: &str,
    reverse: bool,
    bottom: i32,
    leaf_context: &LeafReaderContext,
) -> Result<()> {
    assert_field::<LongComparator>(
        field_name,
        SortFieldType::Long,
        reverse,
        leaf_context,
        |c| {
            c.numeric_comparator_mut().queue_full = true;
            c.numeric_comparator_mut().hits_threshold_reached = true;
            c.bottom = i64::from(bottom);
        },
    )
}

fn assert_int_field(
    field_name: &str,
    reverse: bool,
    bottom: i32,
    leaf_context: &LeafReaderContext,
) -> Result<()> {
    assert_field::<IntComparator>(field_name, SortFieldType::Int, reverse, leaf_context, |c| {
        c.numeric_comparator_mut().queue_full = true;
        c.numeric_comparator_mut().hits_threshold_reached = true;
        c.bottom = bottom;
    })
}

fn assert_float_field(
    field_name: &str,
    reverse: bool,
    bottom: i32,
    leaf_context: &LeafReaderContext,
) -> Result<()> {
    assert_field::<FloatComparator>(
        field_name,
        SortFieldType::Float,
        reverse,
        leaf_context,
        |c| {
            c.numeric_comparator_mut().queue_full = true;
            c.numeric_comparator_mut().hits_threshold_reached = true;
            c.bottom = bottom as f32;
        },
    )
}

fn assert_double_field(
    field_name: &str,
    reverse: bool,
    bottom: i32,
    leaf_context: &LeafReaderContext,
) -> Result<()> {
    assert_field::<DoubleComparator>(
        field_name,
        SortFieldType::Double,
        reverse,
        leaf_context,
        |c| {
            c.numeric_comparator_mut().queue_full = true;
            c.numeric_comparator_mut().hits_threshold_reached = true;
            c.bottom = f64::from(bottom);
        },
    )
}
