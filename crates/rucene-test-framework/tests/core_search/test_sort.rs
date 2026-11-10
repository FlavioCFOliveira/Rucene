//! Port of `lucene/core/src/test/org/apache/lucene/search/TestSort.java`.
//!
//! Very simple tests of sorting.
//!
//! THE RULES:
//! 1. keywords like 'abstract' and 'static' should not appear in this file.
//! 2. each test method should be self-contained and understandable.
//! 3. no test methods should share code with other test methods.
//! 4. no testing of things unrelated to sorting.
//! 5. no tracers.
//! 6. keyword 'class' should appear only once in this file.
//!
//! The documents of the simple sorting tests are lists of [`DocPart`]s,
//! built where Java builds them so that the random draws of `newBytesRef`
//! and `newStringField` keep their place.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    BinaryDocValuesField, Document, DoubleDocValuesField, DoublePoint, FloatDocValuesField,
    FloatPoint, IntPoint, LongPoint, NumericDocValuesField, SortedDocValuesField, Store,
    StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::Term;
use rucene::search::field_comparator::SortValue;
use rucene::search::sort::{MissingValue, Sort, SortField, SortFieldType};
use rucene::search::{double_values_source, long_values_source, MatchAllDocsQuery};
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_equals, class, iae_message, random, test};
use rucene_test_framework::hamcrest::{equal_to, MatcherAssert};

/// `TestSort`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.search.TestSort",
        vec![
            test("testEquals", test_equals),
            test("testStringMissingValues", test_string_missing_values),
            test("testString", test_string),
            test("testStringReverse", test_string_reverse),
            test("testStringVal", test_string_val),
            test("testStringValReverse", test_string_val_reverse),
            test("testInt", test_int),
            test("testIntReverse", test_int_reverse),
            test("testIntMissing", test_int_missing),
            test("testIntMissingLast", test_int_missing_last),
            test("testLong", test_long),
            test("testLongReverse", test_long_reverse),
            test("testLongMissing", test_long_missing),
            test("testLongMissingLast", test_long_missing_last),
            test("testFloat", test_float),
            test("testFloatReverse", test_float_reverse),
            test("testFloatMissing", test_float_missing),
            test("testFloatMissingLast", test_float_missing_last),
            test("testDouble", test_double),
            test("testDoubleSignedZero", test_double_signed_zero),
            test("testDoubleReverse", test_double_reverse),
            test("testDoubleMissing", test_double_missing),
            test("testDoubleMissingLast", test_double_missing_last),
            test("testMultiSort", test_multi_sort),
            test("testRewrite", test_rewrite),
            test("testStringGhost", test_string_ghost),
            test("testIntGhost", test_int_ghost),
            test("testLongGhost", test_long_ghost),
            test("testDoubleGhost", test_double_ghost),
            test("testFloatGhost", test_float_ghost),
        ],
    )
}

/// `a.hashCode()`.
fn hash_code(a: &Sort) -> u64 {
    let mut hasher = DefaultHasher::new();
    a.hash(&mut hasher);
    hasher.finish()
}

/// `private void assertEquals(Sort a, Sort b)`.
fn assert_sort_equals(a: &Sort, b: &Sort) {
    assert_equals(a, b);
    assert_equals(b, a);
    assert_equals(&hash_code(a), &hash_code(b));
}

/// `private void assertDifferent(Sort a, Sort b)`.
fn assert_different(a: &Sort, b: &Sort) {
    assert_not_equals(a, b);
    assert_not_equals(b, a);
    assert_not_equals(&hash_code(a), &hash_code(b));
}

/// `new SortField(field, type, reverse, missingValue)`.
fn sf(
    field: &str,
    t: SortFieldType,
    reverse: bool,
    missing: Option<MissingValue>,
) -> Result<SortField> {
    SortField::new_with_missing(Some(field.to_string()), t, reverse, missing)
}

/// `new Sort(sortField)`.
fn sort_of(sort_field: SortField) -> Result<Sort> {
    Sort::new_fields(vec![sort_field])
}

/// Port of `TestSort.testEquals`.
fn test_equals() -> Result<()> {
    let sort_field1 = sf("foo", SortFieldType::String, false, None)?;
    let sort_field2 = sf("foo", SortFieldType::String, false, None)?;
    assert_sort_equals(&sort_of(sort_field1.clone())?, &sort_of(sort_field2)?);

    let sort_field2 = sf("bar", SortFieldType::String, false, None)?;
    assert_different(&sort_of(sort_field1.clone())?, &sort_of(sort_field2)?);

    let sort_field2 = sf("foo", SortFieldType::Long, false, None)?;
    assert_different(&sort_of(sort_field1.clone())?, &sort_of(sort_field2)?);

    let sort_field2 = sf(
        "foo",
        SortFieldType::String,
        false,
        Some(MissingValue::StringFirst),
    )?;
    assert_different(&sort_of(sort_field1.clone())?, &sort_of(sort_field2)?);

    let sort_field2 = sf("foo", SortFieldType::String, false, None)?;
    assert_sort_equals(&sort_of(sort_field1.clone())?, &sort_of(sort_field2)?);

    let sort_field2 = sf("foo", SortFieldType::String, true, None)?;
    assert_different(&sort_of(sort_field1)?, &sort_of(sort_field2)?);
    Ok(())
}

/// Port of `TestSort.testStringMissingValues`.
fn test_string_missing_values() -> Result<()> {
    // `new SortField("foo", Type.STRING, false, "invalid value")`: Java passes
    // a `String` as the missing value; Rust's missing value is a
    // `MissingValue`, so a value other than STRING_FIRST/STRING_LAST stands
    // for it and takes the same check.
    let message = iae_message(|| {
        sort_of(sf(
            "foo",
            SortFieldType::String,
            false,
            Some(MissingValue::Int(0)),
        )?)
    });
    MatcherAssert::assert_that(
        &message,
        &equal_to(
            "For Type.STRING, missing value must be either STRING_FIRST or STRING_LAST".to_string(),
        ),
    );
    Ok(())
}

/// One field of a document of the simple sorting tests.
enum DocPart {
    /// `new SortedDocValuesField("value", newBytesRef(v))`.
    Sorted(&'static str),
    /// `new BinaryDocValuesField("value", newBytesRef(v))`.
    Binary(&'static str),
    /// `new NumericDocValuesField("value", v)`.
    Numeric(i64),
    /// `new FloatDocValuesField("value", v)`.
    Float(f32),
    /// `new DoubleDocValuesField("value", v)`.
    Double(f64),
    /// `newStringField("value", v, Field.Store.YES)`.
    Stored(&'static str),
}

fn sorted(v: &'static str) -> DocPart {
    DocPart::Sorted(v)
}
fn binary(v: &'static str) -> DocPart {
    DocPart::Binary(v)
}
fn numeric(v: i64) -> DocPart {
    DocPart::Numeric(v)
}
fn float(v: f32) -> DocPart {
    DocPart::Float(v)
}
fn double(v: f64) -> DocPart {
    DocPart::Double(v)
}
fn stored(v: &'static str) -> DocPart {
    DocPart::Stored(v)
}

fn build_doc(parts: &[DocPart]) -> Document {
    let mut doc = Document::new();
    for part in parts {
        match part {
            DocPart::Sorted(v) => doc.add(Box::new(SortedDocValuesField::new(
                "value",
                LuceneTestCase::new_bytes_ref_str(v),
            ))),
            DocPart::Binary(v) => doc.add(Box::new(BinaryDocValuesField::new(
                "value",
                LuceneTestCase::new_bytes_ref_str(v),
            ))),
            DocPart::Numeric(v) => doc.add(Box::new(NumericDocValuesField::new("value", *v))),
            DocPart::Float(v) => doc.add(Box::new(FloatDocValuesField::new("value", *v))),
            DocPart::Double(v) => doc.add(Box::new(DoubleDocValuesField::new("value", *v))),
            DocPart::Stored(v) => doc.add(Box::new(LuceneTestCase::new_string_field(
                "value",
                v,
                Store::YES,
            ))),
        }
    }
    doc
}

/// The body of the simple sorting tests: index `docs` in order, search all
/// documents sorted by `sort_field`, and check the total hits and the stored
/// `value` of each hit in sorted order (`None` for `assertNull`).
fn check_sort(
    docs: &[Vec<DocPart>],
    sort_field: SortField,
    total_hits: i64,
    expected: &[Option<&str>],
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for parts in docs {
        writer.add_document(&build_doc(parts))?;
    }
    let ir = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let sort = sort_of(sort_field)?;

    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 10, &sort)?;
    assert_equals(&total_hits, &td.total_hits.value());
    let stored_fields = searcher.stored_fields()?;
    for (i, value) in expected.iter().enumerate() {
        assert_equals(
            &value.map(|v| v.to_string()),
            &stored_fields
                .document(td.score_docs[i].score_doc.doc)?
                .get("value")
                .map(|s| s.to_string()),
        );
    }

    drop(stored_fields);
    drop(searcher);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// Tests sorting on type string
///
/// Port of `TestSort.testString`.
fn test_string() -> Result<()> {
    // 'bar' comes before 'foo'
    check_sort(
        &[
            vec![sorted("foo"), stored("foo")],
            vec![sorted("bar"), stored("bar")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::String,
            false,
            None,
        )?,
        2,
        &[Some("bar"), Some("foo")],
    )
}

/// Tests reverse sorting on type string
///
/// Port of `TestSort.testStringReverse`.
fn test_string_reverse() -> Result<()> {
    // 'foo' comes after 'bar' in reverse order
    check_sort(
        &[
            vec![sorted("bar"), stored("bar")],
            vec![sorted("foo"), stored("foo")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::String, true, None)?,
        2,
        &[Some("foo"), Some("bar")],
    )
}

/// Tests sorting on type string_val
///
/// Port of `TestSort.testStringVal`.
fn test_string_val() -> Result<()> {
    // 'bar' comes before 'foo'
    check_sort(
        &[
            vec![binary("foo"), stored("foo")],
            vec![binary("bar"), stored("bar")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::StringVal,
            false,
            None,
        )?,
        2,
        &[Some("bar"), Some("foo")],
    )
}

/// Tests reverse sorting on type string_val
///
/// Port of `TestSort.testStringValReverse`.
fn test_string_val_reverse() -> Result<()> {
    // 'foo' comes after 'bar' in reverse order
    check_sort(
        &[
            vec![binary("bar"), stored("bar")],
            vec![binary("foo"), stored("foo")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::StringVal,
            true,
            None,
        )?,
        2,
        &[Some("foo"), Some("bar")],
    )
}

/// Tests sorting on type int
///
/// Port of `TestSort.testInt`.
fn test_int() -> Result<()> {
    // numeric order
    check_sort(
        &[
            vec![numeric(300000), stored("300000")],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Int, false, None)?,
        3,
        &[Some("-1"), Some("4"), Some("300000")],
    )
}

/// Tests sorting on type int in reverse
///
/// Port of `TestSort.testIntReverse`.
fn test_int_reverse() -> Result<()> {
    // reverse numeric order
    check_sort(
        &[
            vec![numeric(300000), stored("300000")],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Int, true, None)?,
        3,
        &[Some("300000"), Some("4"), Some("-1")],
    )
}

/// Tests sorting on type int with a missing value
///
/// Port of `TestSort.testIntMissing`.
fn test_int_missing() -> Result<()> {
    // null is treated as a 0
    check_sort(
        &[
            vec![],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Int, false, None)?,
        3,
        &[Some("-1"), None, Some("4")],
    )
}

/// Tests sorting on type int, specifying the missing value should be treated as Integer.MAX_VALUE
///
/// Port of `TestSort.testIntMissingLast`.
fn test_int_missing_last() -> Result<()> {
    // null is treated as a Integer.MAX_VALUE
    check_sort(
        &[
            vec![],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Int,
            false,
            Some(MissingValue::Int(i32::MAX)),
        )?,
        3,
        &[Some("-1"), Some("4"), None],
    )
}

/// Tests sorting on type long
///
/// Port of `TestSort.testLong`.
fn test_long() -> Result<()> {
    // numeric order
    check_sort(
        &[
            vec![numeric(3000000000), stored("3000000000")],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Long, false, None)?,
        3,
        &[Some("-1"), Some("4"), Some("3000000000")],
    )
}

/// Tests sorting on type long in reverse
///
/// Port of `TestSort.testLongReverse`.
fn test_long_reverse() -> Result<()> {
    // reverse numeric order
    check_sort(
        &[
            vec![numeric(3000000000), stored("3000000000")],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Long, true, None)?,
        3,
        &[Some("3000000000"), Some("4"), Some("-1")],
    )
}

/// Tests sorting on type long with a missing value
///
/// Port of `TestSort.testLongMissing`.
fn test_long_missing() -> Result<()> {
    // null is treated as 0
    check_sort(
        &[
            vec![],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Long, false, None)?,
        3,
        &[Some("-1"), None, Some("4")],
    )
}

/// Tests sorting on type long, specifying the missing value should be treated as Long.MAX_VALUE
///
/// Port of `TestSort.testLongMissingLast`.
fn test_long_missing_last() -> Result<()> {
    // null is treated as Long.MAX_VALUE
    check_sort(
        &[
            vec![],
            vec![numeric(-1), stored("-1")],
            vec![numeric(4), stored("4")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Long,
            false,
            Some(MissingValue::Long(i64::MAX)),
        )?,
        3,
        &[Some("-1"), Some("4"), None],
    )
}

/// Tests sorting on type float
///
/// Port of `TestSort.testFloat`.
fn test_float() -> Result<()> {
    // numeric order
    check_sort(
        &[
            vec![float(30.1), stored("30.1")],
            vec![float(-1.3), stored("-1.3")],
            vec![float(4.2), stored("4.2")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Float, false, None)?,
        3,
        &[Some("-1.3"), Some("4.2"), Some("30.1")],
    )
}

/// Tests sorting on type float in reverse
///
/// Port of `TestSort.testFloatReverse`.
fn test_float_reverse() -> Result<()> {
    // reverse numeric order
    check_sort(
        &[
            vec![float(30.1), stored("30.1")],
            vec![float(-1.3), stored("-1.3")],
            vec![float(4.2), stored("4.2")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Float, true, None)?,
        3,
        &[Some("30.1"), Some("4.2"), Some("-1.3")],
    )
}

/// Tests sorting on type float with a missing value
///
/// Port of `TestSort.testFloatMissing`.
fn test_float_missing() -> Result<()> {
    // null is treated as 0
    check_sort(
        &[
            vec![],
            vec![float(-1.3), stored("-1.3")],
            vec![float(4.2), stored("4.2")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Float, false, None)?,
        3,
        &[Some("-1.3"), None, Some("4.2")],
    )
}

/// Tests sorting on type float, specifying the missing value should be treated as Float.MAX_VALUE
///
/// Port of `TestSort.testFloatMissingLast`.
fn test_float_missing_last() -> Result<()> {
    // null is treated as Float.MAX_VALUE
    check_sort(
        &[
            vec![],
            vec![float(-1.3), stored("-1.3")],
            vec![float(4.2), stored("4.2")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Float,
            false,
            Some(MissingValue::Float(f32::MAX)),
        )?,
        3,
        &[Some("-1.3"), Some("4.2"), None],
    )
}

/// Tests sorting on type double
///
/// Port of `TestSort.testDouble`.
fn test_double() -> Result<()> {
    // numeric order
    check_sort(
        &[
            vec![double(30.1), stored("30.1")],
            vec![double(-1.3), stored("-1.3")],
            vec![double(4.2333333333333), stored("4.2333333333333")],
            vec![double(4.2333333333332), stored("4.2333333333332")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Double,
            false,
            None,
        )?,
        4,
        &[
            Some("-1.3"),
            Some("4.2333333333332"),
            Some("4.2333333333333"),
            Some("30.1"),
        ],
    )
}

/// Tests sorting on type double with +/- zero
///
/// Port of `TestSort.testDoubleSignedZero`.
fn test_double_signed_zero() -> Result<()> {
    // numeric order
    check_sort(
        &[
            vec![double(0.0), stored("+0")],
            vec![double(-0.0), stored("-0")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Double,
            false,
            None,
        )?,
        2,
        &[Some("-0"), Some("+0")],
    )
}

/// Tests sorting on type double in reverse
///
/// Port of `TestSort.testDoubleReverse`.
fn test_double_reverse() -> Result<()> {
    // numeric order
    check_sort(
        &[
            vec![double(30.1), stored("30.1")],
            vec![double(-1.3), stored("-1.3")],
            vec![double(4.2333333333333), stored("4.2333333333333")],
            vec![double(4.2333333333332), stored("4.2333333333332")],
        ],
        SortField::new_with_missing(Some("value".to_string()), SortFieldType::Double, true, None)?,
        4,
        &[
            Some("30.1"),
            Some("4.2333333333333"),
            Some("4.2333333333332"),
            Some("-1.3"),
        ],
    )
}

/// Tests sorting on type double with a missing value
///
/// Port of `TestSort.testDoubleMissing`.
fn test_double_missing() -> Result<()> {
    // null treated as a 0
    check_sort(
        &[
            vec![],
            vec![double(-1.3), stored("-1.3")],
            vec![double(4.2333333333333), stored("4.2333333333333")],
            vec![double(4.2333333333332), stored("4.2333333333332")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Double,
            false,
            None,
        )?,
        4,
        &[
            Some("-1.3"),
            None,
            Some("4.2333333333332"),
            Some("4.2333333333333"),
        ],
    )
}

/// Tests sorting on type double, specifying the missing value should be treated as Double.MAX_VALUE
///
/// Port of `TestSort.testDoubleMissingLast`.
fn test_double_missing_last() -> Result<()> {
    // null treated as Double.MAX_VALUE
    check_sort(
        &[
            vec![],
            vec![double(-1.3), stored("-1.3")],
            vec![double(4.2333333333333), stored("4.2333333333333")],
            vec![double(4.2333333333332), stored("4.2333333333332")],
        ],
        SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Double,
            false,
            Some(MissingValue::Double(f64::MAX)),
        )?,
        4,
        &[
            Some("-1.3"),
            Some("4.2333333333332"),
            Some("4.2333333333333"),
            None,
        ],
    )
}

/// Tests sorting on type double in reverse
///
/// Port of `TestSort.testMultiSort`.
fn test_multi_sort() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    for (value1, value2) in [("foo", 0), ("bar", 1), ("bar", 0), ("foo", 1)] {
        let mut doc = Document::new();
        doc.add(Box::new(SortedDocValuesField::new(
            "value1",
            LuceneTestCase::new_bytes_ref_str(value1),
        )));
        doc.add(Box::new(NumericDocValuesField::new("value2", value2)));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "value1",
            value1,
            Store::YES,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "value2",
            &value2.to_string(),
            Store::YES,
        )));
        writer.add_document(&doc)?;
    }
    let ir = writer.get_reader()?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let sort = Sort::new_fields(vec![
        sf("value1", SortFieldType::String, false, None)?,
        sf("value2", SortFieldType::Long, false, None)?,
    ])?;

    let get = |doc: i32, field: &str| -> Result<Option<String>> {
        Ok(searcher
            .stored_fields()?
            .document(doc)?
            .get(field)
            .map(|s| s.to_string()))
    };
    let some = |s: &str| Some(s.to_string());

    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 10, &sort)?;
    assert_equals(&4, &td.total_hits.value());
    // 'bar' comes before 'foo'
    assert_equals(
        &some("bar"),
        &get(td.score_docs[0].score_doc.doc, "value1")?,
    );
    assert_equals(
        &some("bar"),
        &get(td.score_docs[1].score_doc.doc, "value1")?,
    );
    assert_equals(
        &some("foo"),
        &get(td.score_docs[2].score_doc.doc, "value1")?,
    );
    assert_equals(
        &some("foo"),
        &get(td.score_docs[3].score_doc.doc, "value1")?,
    );
    // 0 comes before 1
    assert_equals(&some("0"), &get(td.score_docs[0].score_doc.doc, "value2")?);
    assert_equals(&some("1"), &get(td.score_docs[1].score_doc.doc, "value2")?);
    assert_equals(&some("0"), &get(td.score_docs[2].score_doc.doc, "value2")?);
    assert_equals(&some("1"), &get(td.score_docs[3].score_doc.doc, "value2")?);

    // Now with overflow
    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 1, &sort)?;
    assert_equals(&4, &td.total_hits.value());
    assert_equals(
        &some("bar"),
        &get(td.score_docs[0].score_doc.doc, "value1")?,
    );
    assert_equals(&some("0"), &get(td.score_docs[0].score_doc.doc, "value2")?);

    drop(searcher);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestSort.testRewrite`.
///
/// `assertSame(sort, sort.rewrite(searcher))`: Rucene's `Sort::rewrite`
/// returns an owned `Sort`, so the unchanged rewrite is compared by equality.
fn test_rewrite() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let result = (|| -> Result<()> {
        let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
        let r = (|| -> Result<()> {
            let reader = writer.get_reader()?;
            let inner = (|| -> Result<()> {
                let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
                let long_source = long_values_source::constant(1);
                let sort = Sort::new_fields(vec![long_values_source::get_sort_field(
                    long_source,
                    false,
                )?])?;

                assert_equals(&sort, &sort.rewrite(&searcher)?);

                let double_source = double_values_source::constant(1.0);
                let sort = Sort::new_fields(vec![double_values_source::get_sort_field(
                    double_source,
                    false,
                )?])?;

                assert_equals(&sort, &sort.rewrite(&searcher)?);
                Ok(())
            })();
            let closed = reader.close();
            inner.and(closed)
        })();
        let closed = writer.close();
        r.and(closed)
    })();
    let closed = dir.close();
    result.and(closed)
}

// Ghost tests make sure that sorting can cope with segments that are missing values while their
// FieldInfo reports that the field exists.

/// The shared body of the ghost tests: `add` puts the doc values (and, when
/// `indexed`, the indexed field) on the one document that is later deleted;
/// the two hits must carry `expected` as their first sort value.
fn do_test_ghost(
    add: impl Fn(&mut Document, bool) -> Result<()>,
    indexed: bool,
    t: SortFieldType,
    expected: SortValue,
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = IndexWriter::new(dir.as_directory(), IndexWriterConfig::new())?;
    let mut doc = Document::new();
    add(&mut doc, indexed)?;
    doc.add(Box::new(StringField::new(
        "id",
        "0".to_string(),
        Store::NO,
    )?));
    writer.add_document(&doc)?;
    writer.add_document(&Document::new())?;
    writer.flush()?;
    writer.add_document(&Document::new())?;
    writer.flush()?;
    writer.delete_documents_by_terms(vec![Term::from_text("id", "0")])?;
    writer.force_merge(1)?;
    let ir = directory_reader::open_with_writer(&*writer)?;
    writer.close()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&ir));
    let sort = sort_of(sf("value", t, false, None)?)?;

    let td = searcher.search_with_sort(Arc::new(MatchAllDocsQuery::instance()), 10, &sort)?;
    assert_equals(&2, &td.total_hits.value());
    let first = |i: usize| -> SortValue {
        // `((FieldDoc) td.scoreDocs[i]).fields[0]`.
        td.score_docs[i].fields.as_ref().expect("sort values")[0].clone()
    };
    assert_equals(&expected, &first(0));
    assert_equals(&expected, &first(1));

    drop(searcher);
    ir.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestSort.testStringGhost`.
fn test_string_ghost() -> Result<()> {
    do_test_string_ghost(true)?;
    do_test_string_ghost(false)
}

/// `private void doTestStringGhost(boolean indexed)`.
fn do_test_string_ghost(indexed: bool) -> Result<()> {
    do_test_ghost(
        |doc, indexed| {
            doc.add(Box::new(SortedDocValuesField::new(
                "value",
                LuceneTestCase::new_bytes_ref_str("foo"),
            )));
            if indexed {
                doc.add(Box::new(LuceneTestCase::new_string_field(
                    "value",
                    "foo",
                    Store::YES,
                )));
            }
            Ok(())
        },
        indexed,
        SortFieldType::String,
        // `assertNull(... fields[0])`.
        SortValue::Null,
    )
}

/// Port of `TestSort.testIntGhost`.
fn test_int_ghost() -> Result<()> {
    do_test_int_ghost(true)?;
    do_test_int_ghost(false)
}

/// `private void doTestIntGhost(boolean indexed)`.
fn do_test_int_ghost(indexed: bool) -> Result<()> {
    do_test_ghost(
        |doc, indexed| {
            doc.add(Box::new(NumericDocValuesField::new("value", 3)));
            if indexed {
                doc.add(Box::new(IntPoint::new("value", &[3])?));
            }
            Ok(())
        },
        indexed,
        SortFieldType::Int,
        SortValue::Int(0),
    )
}

/// Port of `TestSort.testLongGhost`.
fn test_long_ghost() -> Result<()> {
    do_test_long_ghost(true)?;
    do_test_long_ghost(false)
}

/// `private void doTestLongGhost(boolean indexed)`.
fn do_test_long_ghost(indexed: bool) -> Result<()> {
    do_test_ghost(
        |doc, indexed| {
            doc.add(Box::new(NumericDocValuesField::new("value", 3)));
            if indexed {
                doc.add(Box::new(LongPoint::new("value", &[3])?));
            }
            Ok(())
        },
        indexed,
        SortFieldType::Long,
        SortValue::Long(0),
    )
}

/// Port of `TestSort.testDoubleGhost`.
fn test_double_ghost() -> Result<()> {
    do_test_double_ghost(true)?;
    do_test_double_ghost(false)
}

/// `private void doTestDoubleGhost(boolean indexed)`.
fn do_test_double_ghost(indexed: bool) -> Result<()> {
    do_test_ghost(
        |doc, indexed| {
            doc.add(Box::new(DoubleDocValuesField::new("value", 1.25)));
            if indexed {
                doc.add(Box::new(DoublePoint::new("value", &[1.25])?));
            }
            Ok(())
        },
        indexed,
        SortFieldType::Double,
        SortValue::Double(0.0),
    )
}

/// Port of `TestSort.testFloatGhost`.
fn test_float_ghost() -> Result<()> {
    do_test_float_ghost(true)?;
    do_test_float_ghost(false)
}

/// `private void doTestFloatGhost(boolean indexed)`.
fn do_test_float_ghost(indexed: bool) -> Result<()> {
    do_test_ghost(
        |doc, indexed| {
            doc.add(Box::new(FloatDocValuesField::new("value", 1.25)));
            if indexed {
                doc.add(Box::new(FloatPoint::new("value", &[1.25])?));
            }
            Ok(())
        },
        indexed,
        SortFieldType::Float,
        SortValue::Float(0.0),
    )
}
