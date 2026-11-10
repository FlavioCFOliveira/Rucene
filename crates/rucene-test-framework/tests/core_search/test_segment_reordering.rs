//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSegmentReordering.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{
    Document, DoubleField, FloatField, IntField, LongField, NumericDocValuesField,
    SortedNumericDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_policy::NoMergePolicy;
use rucene::index::{IndexReader, SegmentOrder};
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::sorted_numeric_sort_field::SortedNumericSortField;
use rucene::util::NumericUtils;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};

/// `TestSegmentReordering`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSegmentReordering",
        vec![
            test(
                "testSingleValuedNumericSorts",
                test_single_valued_numeric_sorts,
            ),
            test(
                "testMultiValuedSegmentSorts",
                test_multi_valued_segment_sorts,
            ),
            test(
                "testNumericSegmentSortsWithMissingValues",
                test_numeric_segment_sorts_with_missing_values,
            ),
        ],
    )
}

use SortFieldType::{Double as DOUBLE, Float as FLOAT, Int as INT, Long as LONG};

/// `new IndexWriterConfig().setMergePolicy(NoMergePolicy.INSTANCE)`.
fn no_merge_config() -> Result<IndexWriterConfig> {
    let iwc = IndexWriterConfig::new();
    iwc.set_merge_policy(Some(Arc::new(NoMergePolicy::new())))?;
    Ok(iwc)
}

/// Port of `TestSegmentReordering.testSingleValuedNumericSorts`.
fn test_single_valued_numeric_sorts() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), no_merge_config()?)?;

    for i in 0..500 {
        let mut doc = Document::new();
        doc.add(Box::new(LongField::new(
            "long_points",
            i64::from(i),
            Store::NO,
        )));
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "long_skipper",
            i64::from(i),
        )));
        doc.add(Box::new(DoubleField::new(
            "double_points",
            f64::from(i) * 1.5,
            Store::NO,
        )));
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "double_skipper",
            NumericUtils::double_to_sortable_long(f64::from(i) * 1.5),
        )));
        doc.add(Box::new(IntField::new("int_points", i, Store::NO)));
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "int_skipper",
            i64::from(i),
        )));
        doc.add(Box::new(FloatField::new(
            "float_points",
            i as f32 * 1.5,
            Store::NO,
        )));
        doc.add(Box::new(NumericDocValuesField::indexed_field(
            "float_skipper",
            i64::from(NumericUtils::float_to_sortable_int(i as f32 * 1.5)),
        )));
        writer.add_document(&doc)?;
        if i % 125 == 0 {
            writer.commit()?;
        }
    }
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    assert_segment_order(&reader, &sort("long_points", LONG, true)?, &[4, 3, 2, 1, 0])?;
    assert_segment_order(
        &reader,
        &sort("long_skipper", LONG, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(&reader, &sort("int_points", INT, true)?, &[4, 3, 2, 1, 0])?;
    assert_segment_order(&reader, &sort("int_skipper", INT, true)?, &[4, 3, 2, 1, 0])?;
    assert_segment_order(
        &reader,
        &sort("double_points", DOUBLE, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &sort("double_skipper", DOUBLE, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &sort("float_points", FLOAT, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &sort("float_skipper", FLOAT, true)?,
        &[4, 3, 2, 1, 0],
    )?;

    assert_segment_order(
        &reader,
        &sort("long_points", LONG, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &sort("long_skipper", LONG, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(&reader, &sort("int_points", INT, false)?, &[0, 1, 2, 3, 4])?;
    assert_segment_order(&reader, &sort("int_skipper", INT, false)?, &[0, 1, 2, 3, 4])?;
    assert_segment_order(
        &reader,
        &sort("double_points", DOUBLE, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &sort("double_skipper", DOUBLE, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &sort("float_points", FLOAT, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &sort("float_skipper", FLOAT, false)?,
        &[0, 1, 2, 3, 4],
    )?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestSegmentReordering.testMultiValuedSegmentSorts`.
fn test_multi_valued_segment_sorts() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), no_merge_config()?)?;

    let mut i = 0;
    while i < 1000 {
        let mut doc = Document::new();
        doc.add(Box::new(LongField::new(
            "long_points",
            i64::from(i),
            Store::NO,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "long_skipper",
            i64::from(i),
        )));
        doc.add(Box::new(LongField::new(
            "long_points",
            i64::from(i + 1),
            Store::NO,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "long_skipper",
            i64::from(i + 1),
        )));

        doc.add(Box::new(DoubleField::new(
            "double_points",
            f64::from(i) * 1.5,
            Store::NO,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "double_skipper",
            NumericUtils::double_to_sortable_long(f64::from(i) * 1.5),
        )));
        doc.add(Box::new(DoubleField::new(
            "double_points",
            f64::from(i) * 1.5 + 1.0,
            Store::NO,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "double_skipper",
            NumericUtils::double_to_sortable_long(f64::from(i) * 1.5 + 1.0),
        )));
        doc.add(Box::new(IntField::new("int_points", i, Store::NO)));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "int_skipper",
            i64::from(i),
        )));
        doc.add(Box::new(IntField::new("int_points", i + 1, Store::NO)));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "int_skipper",
            i64::from(i + 1),
        )));

        doc.add(Box::new(FloatField::new(
            "float_points",
            i as f32 * 1.5,
            Store::NO,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "float_skipper",
            i64::from(NumericUtils::float_to_sortable_int(i as f32 * 1.5)),
        )));
        doc.add(Box::new(FloatField::new(
            "float_points",
            i as f32 * 1.5 + 1.0,
            Store::NO,
        )));
        doc.add(Box::new(SortedNumericDocValuesField::indexed_field(
            "float_skipper",
            i64::from(NumericUtils::float_to_sortable_int(i as f32 * 1.5 + 1.0)),
        )));

        writer.add_document(&doc)?;
        if i % 250 == 0 {
            writer.commit()?;
        }
        i += 2;
    }
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    assert_segment_order(
        &reader,
        &msort("long_points", LONG, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &msort("long_skipper", LONG, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(&reader, &msort("int_points", INT, true)?, &[4, 3, 2, 1, 0])?;
    assert_segment_order(&reader, &msort("int_skipper", INT, true)?, &[4, 3, 2, 1, 0])?;
    assert_segment_order(
        &reader,
        &msort("double_points", DOUBLE, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &msort("double_skipper", DOUBLE, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &msort("float_points", FLOAT, true)?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &msort("float_skipper", FLOAT, true)?,
        &[4, 3, 2, 1, 0],
    )?;

    assert_segment_order(
        &reader,
        &msort("long_points", LONG, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &msort("long_skipper", LONG, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(&reader, &msort("int_points", INT, false)?, &[0, 1, 2, 3, 4])?;
    assert_segment_order(
        &reader,
        &msort("int_skipper", INT, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &msort("double_points", DOUBLE, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &msort("double_skipper", DOUBLE, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &msort("float_points", FLOAT, false)?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &msort("float_skipper", FLOAT, false)?,
        &[0, 1, 2, 3, 4],
    )?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `private static Sort msort(String field, SortField.Type type, boolean reverse)`.
fn msort(field: &str, t: SortFieldType, reverse: bool) -> Result<Sort> {
    Sort::new_fields(vec![SortedNumericSortField::with_reverse(
        field, t, reverse,
    )?
    .into_sort_field()])
}

/// `private static Sort sort(String field, SortField.Type type, boolean reverse)`.
fn sort(field: &str, t: SortFieldType, reverse: bool) -> Result<Sort> {
    sort_missing(field, t, reverse, None)
}

/// `private static Sort sort(String field, SortField.Type type, boolean reverse, Object missingValue)`.
fn sort_missing(
    field: &str,
    t: SortFieldType,
    reverse: bool,
    missing_value: Option<MissingValue>,
) -> Result<Sort> {
    let sf = SortField::new_with_missing(Some(field.to_string()), t, reverse, missing_value)?;
    Sort::new_fields(vec![sf])
}

/// Port of `TestSegmentReordering.testNumericSegmentSortsWithMissingValues`.
fn test_numeric_segment_sorts_with_missing_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), no_merge_config()?)?;
    for i in 0..500 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "string",
            "foo".to_string(),
            Store::NO,
        )?));
        if i != 200 {
            doc.add(Box::new(LongField::new(
                "long_points",
                i64::from(i),
                Store::NO,
            )));
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "long_skipper",
                i64::from(i),
            )));
            doc.add(Box::new(DoubleField::new(
                "double_points",
                f64::from(i) * 1.5,
                Store::NO,
            )));
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "double_skipper",
                NumericUtils::double_to_sortable_long(f64::from(i) * 1.5),
            )));
            doc.add(Box::new(IntField::new("int_points", i, Store::NO)));
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "int_skipper",
                i64::from(i),
            )));
            doc.add(Box::new(FloatField::new(
                "float_points",
                i as f32 * 1.5,
                Store::NO,
            )));
            doc.add(Box::new(NumericDocValuesField::indexed_field(
                "float_skipper",
                i64::from(NumericUtils::float_to_sortable_int(i as f32 * 1.5)),
            )));
        }
        writer.add_document(&doc)?;
        if i % 125 == 0 {
            writer.commit()?;
        }
    }
    writer.close()?;

    let reader: Arc<dyn IndexReader> = directory_reader::open(dir.as_directory())?;

    let s = |field: &str, t: SortFieldType, reverse: bool, missing: MissingValue| {
        sort_missing(field, t, reverse, Some(missing))
    };
    use MissingValue::{Double as D, Float as F, Int as I, Long as L};

    assert_segment_order(
        &reader,
        &s("long_points", LONG, true, L(i64::MAX))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("long_points", LONG, false, L(i64::MAX))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("long_points", LONG, true, L(i64::MIN))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("long_points", LONG, false, L(i64::MIN))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("long_skipper", LONG, true, L(i64::MAX))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("long_skipper", LONG, false, L(i64::MAX))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("long_skipper", LONG, true, L(i64::MIN))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("long_skipper", LONG, false, L(i64::MIN))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("int_points", INT, true, I(i32::MAX))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("int_points", INT, false, I(i32::MAX))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("int_points", INT, true, I(i32::MIN))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("int_points", INT, false, I(i32::MIN))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("int_skipper", INT, true, I(i32::MAX))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("int_skipper", INT, false, I(i32::MAX))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("int_skipper", INT, true, I(i32::MIN))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("int_skipper", INT, false, I(i32::MIN))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("double_points", DOUBLE, true, D(f64::INFINITY))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("double_points", DOUBLE, false, D(f64::INFINITY))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("double_points", DOUBLE, true, D(f64::NEG_INFINITY))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("double_points", DOUBLE, false, D(f64::NEG_INFINITY))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("double_skipper", DOUBLE, true, D(f64::INFINITY))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("double_skipper", DOUBLE, false, D(f64::INFINITY))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("double_skipper", DOUBLE, true, D(f64::NEG_INFINITY))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("double_skipper", DOUBLE, false, D(f64::NEG_INFINITY))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("float_points", FLOAT, true, F(f32::INFINITY))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("float_points", FLOAT, false, F(f32::INFINITY))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("float_points", FLOAT, true, F(f32::NEG_INFINITY))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("float_points", FLOAT, false, F(f32::NEG_INFINITY))?,
        &[2, 0, 1, 3, 4],
    )?;

    assert_segment_order(
        &reader,
        &s("float_skipper", FLOAT, true, F(f32::INFINITY))?,
        &[2, 4, 3, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("float_skipper", FLOAT, false, F(f32::INFINITY))?,
        &[0, 1, 2, 3, 4],
    )?;
    assert_segment_order(
        &reader,
        &s("float_skipper", FLOAT, true, F(f32::NEG_INFINITY))?,
        &[4, 3, 2, 1, 0],
    )?;
    assert_segment_order(
        &reader,
        &s("float_skipper", FLOAT, false, F(f32::NEG_INFINITY))?,
        &[2, 0, 1, 3, 4],
    )?;

    reader.close()?;
    dir.close()?;
    Ok(())
}

/// `private void assertSegmentOrder(IndexReader reader, Sort sort, int... expectedOrds)`.
fn assert_segment_order(
    reader: &Arc<dyn IndexReader>,
    sort: &Sort,
    expected_ords: &[usize],
) -> Result<()> {
    let leaves = Arc::clone(reader).leaves()?;
    assert_equals(&expected_ords.len(), &leaves.len());
    let reordered_reader = SegmentOrder::from_sort(sort).reorder(Arc::clone(reader))?;

    let reordered_leaves = reordered_reader.leaves()?;
    for i in 0..expected_ords.len() {
        // `assertEquals(reader, reader)`: `LeafReader` keeps `Object.equals`.
        Assert::assert_true(std::ptr::addr_eq(
            Arc::as_ptr(&reordered_leaves[i].leaf_reader()),
            Arc::as_ptr(&leaves[expected_ords[i]].leaf_reader()),
        ));
    }
    Ok(())
}
