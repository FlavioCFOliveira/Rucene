//! Port of `lucene/core/src/test/org/apache/lucene/document/TestKeywordField.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, KeywordField, Store};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexReader, IndexableField};
use rucene::util::BytesRef;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_null, class, random, test};

/// `TestKeywordField`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestKeywordField",
        vec![
            test("testSetBytesValue", test_set_bytes_value),
            test("testSetStringValue", test_set_string_value),
            test("testIndexBytesValue", test_index_bytes_value),
            test("testIndexStringValue", test_index_string_value),
            test("testValueClone", test_value_clone),
        ],
    )
}

/// `newBytesRef(s)`.
fn new_bytes_ref(s: &str) -> BytesRef {
    LuceneTestCase::new_bytes_ref_str(s)
}

/// Port of `TestKeywordField.testSetBytesValue`.
fn test_set_bytes_value() -> Result<()> {
    let fields = vec![
        KeywordField::new_with_bytes("name", new_bytes_ref("value"), Store::NO)?,
        KeywordField::new_with_bytes("name", new_bytes_ref("value"), Store::YES)?,
    ];
    for field in fields {
        assert_equals(&Some(new_bytes_ref("value")), &field.binary_value());
        assert_null(&field.string_value());
        if field.field_type().stored() {
            assert_equals(
                &new_bytes_ref("value"),
                field.stored_value()?.expect("stored").binary_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
        field.set_bytes_value(new_bytes_ref("value2"))?;
        assert_equals(&Some(new_bytes_ref("value2")), &field.binary_value());
        assert_null(&field.string_value());
        if field.field_type().stored() {
            assert_equals(
                &new_bytes_ref("value2"),
                field.stored_value()?.expect("stored").binary_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// Port of `TestKeywordField.testSetStringValue`.
fn test_set_string_value() -> Result<()> {
    let fields = vec![
        KeywordField::new("name", "value".to_string(), Store::NO)?,
        KeywordField::new("name", "value".to_string(), Store::YES)?,
    ];
    for field in fields {
        assert_equals(&Some("value".to_string()), &field.string_value());
        assert_equals(&Some(new_bytes_ref("value")), &field.binary_value());
        if field.field_type().stored() {
            assert_equals(
                "value",
                field.stored_value()?.expect("stored").string_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
        field.set_string_value("value2".to_string())?;
        assert_equals(&Some("value2".to_string()), &field.string_value());
        assert_equals(&Some(new_bytes_ref("value2")), &field.binary_value());
        if field.field_type().stored() {
            assert_equals(
                "value2",
                field.stored_value()?.expect("stored").string_value()?,
            );
        } else {
            assert_null(&field.stored_value()?);
        }
    }
    Ok(())
}

/// The shared body of `testIndexBytesValue` and `testIndexStringValue`:
/// index `field` alone, then check its term, its doc values and its stored
/// value with `check_stored`.
fn index_one(field: KeywordField, check_stored: impl FnOnce(&Document)) -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let w = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config(),
    )?;
    // w.addDocument(Collections.singleton(field))
    let mut doc = Document::new();
    doc.add(Box::new(field));
    w.add_document(&doc)?;
    let reader: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*w)?;
    w.close()?;
    let leaf = LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader))?;
    let mut terms = leaf.terms("field")?.expect("terms").iterator()?;
    assert_equals(&Some(BytesRef::from_char_sequence("value")), &terms.next()?);
    assert_null(&terms.next()?);

    let mut values = leaf
        .get_sorted_set_doc_values("field")?
        .expect("doc values");
    Assert::assert_true(values.advance_exact(0)?);
    assert_equals(&1, &values.doc_value_count()?);
    assert_equals(&0i64, &values.next_ord()?);
    assert_equals(
        &BytesRef::from_char_sequence("value"),
        &values.lookup_ord(0)?,
    );

    let stored_doc = leaf.stored_fields()?.document(0)?;
    check_stored(&stored_doc);
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestKeywordField.testIndexBytesValue`.
fn test_index_bytes_value() -> Result<()> {
    index_one(
        KeywordField::new_with_bytes("field", new_bytes_ref("value"), Store::YES)?,
        |stored_doc| {
            assert_equals(
                &Some(BytesRef::from_char_sequence("value")),
                &stored_doc.get_binary_value("field"),
            )
        },
    )
}

/// Port of `TestKeywordField.testIndexStringValue`.
fn test_index_string_value() -> Result<()> {
    index_one(
        KeywordField::new("field", "value".to_string(), Store::YES)?,
        |stored_doc| assert_equals(&Some("value".to_string()), &stored_doc.get("field")),
    )
}

/// Port of `TestKeywordField.testValueClone`.
fn test_value_clone() -> Result<()> {
    let mut values: Vec<BytesRef> = Vec::with_capacity(100);
    for _i in 0..100 {
        let s = TestUtil::random_simple_string_between(&*random(), 10, 20);
        values.push(BytesRef::from_char_sequence(&s));
    }

    // Make sure we don't modify the input values array.
    let expected = values.clone();
    // Java passes the list itself; Rucene's `newSetQuery` takes the values
    // by value, so it is handed a copy and the caller's list is compared.
    KeywordField::new_set_query("f", values.clone())?;
    assert_equals(&expected, &values);
    Ok(())
}
