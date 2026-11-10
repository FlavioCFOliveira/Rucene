//! Port of `lucene/core/src/test/org/apache/lucene/document/TestBinaryDocument.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Field, FieldType, StoredField};
use rucene::error::Result;
use rucene::store::InputStreamDataInput;
use rucene::util::BytesRef;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, assert_not_null, class, random, test};

/// `TestBinaryDocument`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestBinaryDocument",
        vec![
            test("testBinaryFieldInIndex", test_binary_field_in_index),
            test(
                "testBinaryFieldFromDataInputInIndex",
                test_binary_field_from_data_input_in_index,
            ),
        ],
    )
}

/// `String binaryValStored`.
const BINARY_VAL_STORED: &str = "this text will be stored as a byte array in the index";
/// `String binaryValCompressed` (declared, unused by Java's tests).
#[allow(dead_code)]
const BINARY_VAL_COMPRESSED: &str =
    "this text will be also stored and compressed as a byte array in the index";

/// The shared tail of both tests: index `doc`, read it back and check its
/// two fields.
fn index_and_check(doc: Document) -> Result<()> {
    /* test for field count */
    assert_equals(&2, &doc.get_fields().len());

    /* add the doc to a ram index */
    let mut dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    writer.add_document(&doc)?;

    /* open a reader and fetch the document */
    let reader = writer.get_reader()?;
    let doc_from_reader = reader.stored_fields()?.document(0)?;

    /* fetch the binary stored field and compare its content with the original one */
    let bytes: Option<BytesRef> = doc_from_reader.get_binary_value("binaryStored");
    assert_not_null(&bytes);
    let bytes = bytes.expect("not null");
    let binary_fld_stored_test =
        String::from_utf8_lossy(&bytes.bytes[bytes.offset..bytes.offset + bytes.length])
            .into_owned();
    Assert::assert_true(binary_fld_stored_test == BINARY_VAL_STORED);

    /* fetch the string field and compare its content with the original one */
    let string_fld_stored_test = doc_from_reader.get("stringStored");
    Assert::assert_true(string_fld_stored_test.as_deref() == Some(BINARY_VAL_STORED));

    writer.close()?;
    reader.close()?;
    dir.close()?;
    Ok(())
}

/// Port of `TestBinaryDocument.testBinaryFieldInIndex`.
fn test_binary_field_in_index() -> Result<()> {
    let mut ft = FieldType::new();
    ft.set_stored(true)?;
    let binary_fld_stored = StoredField::new_bytes(
        "binaryStored",
        BytesRef::new(BINARY_VAL_STORED.as_bytes().to_vec()),
    )?;
    let string_fld_stored = Field::new("stringStored", BINARY_VAL_STORED.to_string(), ft)?;

    let mut doc = Document::new();
    doc.add(Box::new(binary_fld_stored));
    doc.add(Box::new(string_fld_stored));
    index_and_check(doc)
}

/// Port of `TestBinaryDocument.testBinaryFieldFromDataInputInIndex`.
///
/// Java wraps a `ByteArrayDataInput` in a `StoredFieldDataInput`; Rucene's
/// `StoredField::new_stored_input` takes the owned input and its length (the
/// two members of `StoredFieldDataInput`), so the bytes are read through an
/// owned `InputStreamDataInput` over them.
fn test_binary_field_from_data_input_in_index() -> Result<()> {
    let mut ft = FieldType::new();
    ft.set_stored(true)?;
    let byte_array = BINARY_VAL_STORED.as_bytes().to_vec();
    let length = byte_array.len() as i32;
    let binary_fld_stored = StoredField::new_stored_input(
        "binaryStored",
        Some(Box::new(InputStreamDataInput::new(std::io::Cursor::new(
            byte_array,
        )))),
        length,
    )?;
    let string_fld_stored = Field::new("stringStored", BINARY_VAL_STORED.to_string(), ft)?;

    let mut doc = Document::new();
    doc.add(Box::new(binary_fld_stored));
    doc.add(Box::new(string_fld_stored));
    index_and_check(doc)
}
