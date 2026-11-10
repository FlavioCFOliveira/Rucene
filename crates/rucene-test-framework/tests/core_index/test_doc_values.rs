//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocValues.java`.
//!
//! Tests helper methods in DocValues

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{
    BinaryDocValuesField, Document, NumericDocValuesField, SortedDocValuesField,
    SortedNumericDocValuesField, SortedSetDocValuesField, Store, StringField,
};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::doc_values::DocValues;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, IndexableField, LeafReader};
use rucene::store::Directory;
use rucene::util::{BytesRef, IOUtils};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestDocValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocValues",
        vec![
            test("testEmptyIndex", test_empty_index),
            test("testMisconfiguredField", test_misconfigured_field),
            test("testNumericField", test_numeric_field),
            test("testBinaryField", test_binary_field),
            test("testSortedField", test_sorted_field),
            test("testSortedSetField", test_sorted_set_field),
            test("testSortedNumericField", test_sorted_numeric_field),
            test(
                "testAddNullNumericDocValues",
                test_add_null_numeric_doc_values,
            ),
        ],
    )
}

const ISE: &str = "java.lang.IllegalStateException";

/// `new IndexWriter(dir, newIndexWriterConfig(null))`.
fn null_analyzer_writer(dir: &Arc<dyn Directory>) -> Result<Arc<IndexWriter>> {
    IndexWriter::new(
        Arc::clone(dir),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )
}

/// The five `DocValues.get*` accessors.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Getter {
    Binary,
    Numeric,
    Sorted,
    SortedSet,
    SortedNumeric,
}

/// `DocValues.get<getter>(r, field)`: whether it returned an instance.
fn get(getter: Getter, r: &dyn LeafReader, field: &str) -> Result<()> {
    match getter {
        Getter::Binary => DocValues::get_binary(r, field).map(|_| ()),
        Getter::Numeric => DocValues::get_numeric(r, field).map(|_| ()),
        Getter::Sorted => DocValues::get_sorted(r, field).map(|_| ()),
        Getter::SortedSet => DocValues::get_sorted_set(r, field).map(|_| ()),
        Getter::SortedNumeric => DocValues::get_sorted_numeric(r, field).map(|_| ()),
    }
}

/// The body every field test shares: index one document holding `field`
/// (or no field), then `assertNotNull(DocValues.get…)` for each of `ok` and
/// `expectThrows(IllegalStateException.class, …)` for each of `errors`, on
/// `name`.
fn check(
    field: Option<Box<dyn IndexableField>>,
    name: &str,
    ok: &[Getter],
    errors: &[Getter],
) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = null_analyzer_writer(&dir.as_directory())?;
    let mut doc = Document::new();
    if let Some(field) = field {
        doc.add(field);
    }
    iw.add_document(&doc)?;
    let dr = directory_reader::open_with_writer(iw.as_ref())?;
    let r = LuceneTestCase::get_only_leaf_reader(Arc::clone(&dr) as Arc<dyn IndexReader>)?;

    // ok
    for getter in ok {
        // `assertNotNull`: the accessor returns an instance, never `null`.
        get(*getter, r.as_ref(), name)?;
    }

    // errors
    for getter in errors {
        expect_throws(ISE, || get(*getter, r.as_ref(), name));
    }

    dr.close()?;
    iw.close()?;
    drop((r, dr, iw));
    dir.close()
}

/// If the field doesn't exist, we return empty instances: it can easily happen that a segment just
/// doesn't have any docs with the field.
///
/// Port of `TestDocValues.testEmptyIndex`.
fn test_empty_index() -> Result<()> {
    use Getter::*;
    check(
        None,
        "bogus",
        &[Binary, Numeric, Sorted, SortedSet, SortedNumeric],
        &[],
    )
}

/// field just doesnt have any docvalues at all: exception
///
/// Port of `TestDocValues.testMisconfiguredField`.
fn test_misconfigured_field() -> Result<()> {
    use Getter::*;
    check(
        Some(Box::new(StringField::new(
            "foo",
            "bar".to_string(),
            Store::NO,
        )?)),
        "foo",
        &[],
        &[Binary, Numeric, Sorted, SortedSet, SortedNumeric],
    )
}

/// field with numeric docvalues
///
/// Port of `TestDocValues.testNumericField`.
fn test_numeric_field() -> Result<()> {
    use Getter::*;
    check(
        Some(Box::new(NumericDocValuesField::new("foo", 3))),
        "foo",
        &[Numeric, SortedNumeric],
        &[Binary, Sorted, SortedSet],
    )
}

/// field with binary docvalues
///
/// Port of `TestDocValues.testBinaryField`.
fn test_binary_field() -> Result<()> {
    use Getter::*;
    check(
        Some(Box::new(BinaryDocValuesField::new(
            "foo",
            BytesRef::from_char_sequence("bar"),
        ))),
        "foo",
        &[Binary],
        &[Numeric, Sorted, SortedSet, SortedNumeric],
    )
}

/// field with sorted docvalues
///
/// Port of `TestDocValues.testSortedField`.
fn test_sorted_field() -> Result<()> {
    use Getter::*;
    check(
        Some(Box::new(SortedDocValuesField::new(
            "foo",
            BytesRef::from_char_sequence("bar"),
        ))),
        "foo",
        &[Sorted, SortedSet],
        &[Binary, Numeric, SortedNumeric],
    )
}

/// field with sortedset docvalues
///
/// Port of `TestDocValues.testSortedSetField`.
fn test_sorted_set_field() -> Result<()> {
    use Getter::*;
    check(
        Some(Box::new(SortedSetDocValuesField::new(
            "foo",
            BytesRef::from_char_sequence("bar"),
        ))),
        "foo",
        &[SortedSet],
        &[Binary, Numeric, Sorted, SortedNumeric],
    )
}

/// field with sortednumeric docvalues
///
/// Port of `TestDocValues.testSortedNumericField`.
fn test_sorted_numeric_field() -> Result<()> {
    use Getter::*;
    check(
        Some(Box::new(SortedNumericDocValuesField::new("foo", 3))),
        "foo",
        &[SortedNumeric],
        &[Binary, Numeric, Sorted, SortedSet],
    )
}

/// Port of `TestDocValues.testAddNullNumericDocValues`.
fn test_add_null_numeric_doc_values() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = null_analyzer_writer(&dir.as_directory())?;
    let mut doc = Document::new();
    if random().next_boolean() {
        doc.add(Box::new(NumericDocValuesField::new_nullable("foo", None)));
    } else {
        doc.add(Box::new(BinaryDocValuesField::new_nullable("foo", None)));
    }
    let iae = expect_throws("java.lang.IllegalArgumentException", || {
        iw.add_document(&doc)
    });
    assert_equals(
        &Some("field=\"foo\": null value not allowed"),
        &iae.message(),
    );
    // `IOUtils.close(iw, dir)`.
    let dir_handle = &mut dir;
    IOUtils::close([
        Some(Box::new(|| iw.close()) as Box<dyn FnOnce() -> Result<()>>),
        Some(Box::new(|| dir_handle.close())),
    ])
}
