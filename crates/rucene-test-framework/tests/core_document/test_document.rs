//! Port of `lucene/core/src/test/org/apache/lucene/document/TestDocument.java`:
//! tests the `Document` class.

use rucene::util::atomic_ref_cell::AtomicRefCell;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::StringCharReader;
use rucene::document::{
    Document, Field, FieldType, NumericValue, Store, StoredField, StringField, TextField,
};
use rucene::error::Result;
use rucene::index::{IndexOptions, IndexReader, IndexableField, Term};
use rucene::search::{PhraseQuery, Query, TermQuery};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockTokenizer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::hamcrest::{
    assert_array_containing, assert_array_with_size, assert_contains_in_any_order, assert_empty,
    assert_equal_to, assert_has_size, assert_not_equal_to, assert_not_null_value,
    assert_null_value,
};
use crate::core_support::{assert_array_equals, assert_equals, class, random, test, IAE};

/// `TestDocument`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestDocument",
        vec![
            test("testBinaryField", test_binary_field),
            test("testRemoveForNewDocument", test_remove_for_new_document),
            test("testConstructorExceptions", test_constructor_exceptions),
            test("testClearDocument", test_clear_document),
            test("testGetFieldsImmutable", test_get_fields_immutable),
            test(
                "testGetValuesForNewDocument",
                test_get_values_for_new_document,
            ),
            test(
                "testGetValuesForIndexedDocument",
                test_get_values_for_indexed_document,
            ),
            test("testGetValues", test_get_values),
            test(
                "testPositionIncrementMultiFields",
                test_position_increment_multi_fields,
            ),
            test("testFieldSetValue", test_field_set_value),
            test("testInvalidFields", test_invalid_fields),
            test("testNumericFieldAsString", test_numeric_field_as_string),
        ],
    )
}

const BINARY_VAL: &str = "this text will be stored as a byte array in the index";
const BINARY_VAL2: &str = "this text will be also stored as a byte array in the index";

/// `new RandomIndexWriter(random(), dir)`.
fn random_writer(
    dir: &rucene_test_framework::util::lucene_test_case::TestDirectory,
) -> Result<RandomIndexWriter> {
    RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())
}

/// Port of `testBinaryField`.
fn test_binary_field() -> Result<()> {
    let mut doc = Document::new();

    let ft = FieldType::new();
    ft.set_stored(true)?;
    let string_fld = Field::new("string", BINARY_VAL.to_string(), ft)?;
    let binary_fld =
        StoredField::new_bytes("binary", BytesRef::new(BINARY_VAL.as_bytes().to_vec()))?;
    let binary_fld2 =
        StoredField::new_bytes("binary", BytesRef::new(BINARY_VAL2.as_bytes().to_vec()))?;

    // Java checks `binaryFld` after adding it; the port checks before
    // `doc.add` moves it (the field is not mutated by the add).
    assert_not_null_value(binary_fld.binary_value().is_none());
    Assert::assert_true(binary_fld.field_type().stored());
    assert_equals(
        &IndexOptions::NONE,
        &binary_fld.field_type().index_options(),
    );

    doc.add(Box::new(string_fld));
    doc.add(Box::new(binary_fld));

    assert_has_size(doc.get_fields().len(), 2);

    let mut binary_test = doc
        .get_binary_value("binary")
        .expect("binary")
        .utf8_to_string()?;
    assert_equal_to(binary_test.as_str(), BINARY_VAL);

    let string_test = doc.get("string").unwrap_or_default();
    assert_equal_to(binary_test.as_str(), string_test.as_str());

    doc.add(Box::new(binary_fld2));

    assert_has_size(doc.get_fields().len(), 3);

    let binary_tests = doc.get_binary_values("binary");

    assert_array_with_size(binary_tests.len(), 2);

    binary_test = binary_tests[0].utf8_to_string()?;
    let binary_test2 = binary_tests[1].utf8_to_string()?;

    assert_not_equal_to(binary_test.as_str(), binary_test2.as_str());

    assert_equal_to(binary_test.as_str(), BINARY_VAL);
    assert_equal_to(binary_test2.as_str(), BINARY_VAL2);

    doc.remove_field("string");
    assert_has_size(doc.get_fields().len(), 2);

    doc.remove_fields("binary");
    assert_empty(doc.get_fields().len());
    Ok(())
}

/// Port of `testRemoveForNewDocument`: `Document.removeField(String)` on a
/// brand new document that has not been indexed yet.
fn test_remove_for_new_document() -> Result<()> {
    let mut doc = make_document_with_fields()?;
    assert_has_size(doc.get_fields().len(), 10);
    doc.remove_fields("keyword");
    assert_has_size(doc.get_fields().len(), 8);
    doc.remove_fields("doesnotexists"); // removing non-existing fields is
                                        // silently ignored
    doc.remove_fields("keyword"); // removing a field more than once
    assert_has_size(doc.get_fields().len(), 8);
    doc.remove_field("text");
    assert_has_size(doc.get_fields().len(), 7);
    doc.remove_field("text");
    assert_has_size(doc.get_fields().len(), 6);
    doc.remove_field("text");
    assert_has_size(doc.get_fields().len(), 6);
    doc.remove_field("doesnotexists"); // removing non-existing fields is
                                       // silently ignored
    assert_has_size(doc.get_fields().len(), 6);
    doc.remove_fields("unindexed");
    assert_has_size(doc.get_fields().len(), 4);
    doc.remove_fields("unstored");
    assert_has_size(doc.get_fields().len(), 2);
    doc.remove_fields("doesnotexists"); // removing non-existing fields is
                                        // silently ignored
    assert_has_size(doc.get_fields().len(), 2);

    doc.remove_fields("indexed_not_tokenized");
    assert_empty(doc.get_fields().len());
    Ok(())
}

/// Port of `testConstructorExceptions`.
fn test_constructor_exceptions() -> Result<()> {
    let ft = FieldType::new();
    ft.set_stored(true)?;
    Field::new("name", "value".to_string(), ft.clone())?; // okay
    StringField::new("name", "value".to_string(), Store::NO)?; // okay
    expect_throws(IAE, || {
        Field::new("name", "value".to_string(), FieldType::new())
    });

    let mut dir = LuceneTestCase::new_directory();
    let w = random_writer(&dir)?;
    Field::new("name", "value".to_string(), ft)?; // okay
    let mut doc = Document::new();
    let ft2 = FieldType::new();
    ft2.set_stored(true)?;
    ft2.set_store_term_vectors(true)?;
    doc.add(Box::new(Field::new("name", "value".to_string(), ft2)?));
    expect_throws(IAE, || w.add_document(&doc));
    w.close()?;
    dir.close()
}

/// Port of `testClearDocument`.
fn test_clear_document() -> Result<()> {
    let mut doc = make_document_with_fields()?;
    assert_has_size(doc.get_fields().len(), 10);
    doc.clear();
    assert_empty(doc.get_fields().len());
    Ok(())
}

/// Port of `testGetFieldsImmutable`: `Document.getFields()` returns an
/// immutable list.
///
/// **Divergence from Lucene 10.5.0.** Java checks immutability at run time:
/// `fields.add(…)` and `fields.clear()` throw `UnsupportedOperationException`.
/// The port's `get_fields` returns a shared slice `&[Box<dyn IndexableField>]`,
/// which has no `add` or `clear`: the two mutations do not compile, so the
/// guarantee is checked by the type below instead of at run time.
fn test_get_fields_immutable() -> Result<()> {
    let doc = make_document_with_fields()?;
    assert_has_size(doc.get_fields().len(), 10);
    let fields: &[Arc<dyn IndexableField>] = doc.get_fields();
    // fields.add(new StringField("name", "value", Field.Store.NO)) and
    // fields.clear(): statically rejected (see the divergence note).
    assert_has_size(fields.len(), 10);
    Ok(())
}

/// Port of `testGetValuesForNewDocument`: `Document.getValues(String)` on a
/// brand new document that has not been indexed yet.
fn test_get_values_for_new_document() -> Result<()> {
    do_assert(&make_document_with_fields()?, false);
    Ok(())
}

/// Port of `testGetValuesForIndexedDocument`: `Document.getValues(String)`
/// on a document retrieved from an index.
fn test_get_values_for_indexed_document() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;
    writer.add_document(&make_document_with_fields()?)?;
    let reader = writer.get_reader()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    // search for something that does exist
    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("keyword", "test1")));

    // ensure that queries return expected results without DateFilter first
    let hits = searcher.search(query, 1000)?.score_docs;
    assert_array_with_size(hits.len(), 1);

    do_assert(&searcher.stored_fields()?.document(hits[0].doc)?, true);
    writer.close()?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// Port of `testGetValues`.
fn test_get_values() -> Result<()> {
    let doc = make_document_with_fields()?;
    let pair = vec!["test1".to_string(), "test2".to_string()];
    assert_array_equals(&pair, &doc.get_values("keyword"));
    assert_array_equals(&pair, &doc.get_values("text"));
    assert_array_equals(&pair, &doc.get_values("unindexed"));
    assert_array_equals(&Vec::<String>::new(), &doc.get_values("nope"));
    Ok(())
}

/// Port of `testPositionIncrementMultiFields`.
fn test_position_increment_multi_fields() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;
    writer.add_document(&make_document_with_fields()?)?;
    let reader = writer.get_reader()?;

    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query: Arc<dyn Query> = Arc::new(PhraseQuery::from_strings(
        "indexed_not_tokenized",
        &["test1", "test2"],
    )?);

    let hits = searcher.search(query, 1000)?.score_docs;
    assert_array_with_size(hits.len(), 1);

    do_assert(&searcher.stored_fields()?.document(hits[0].doc)?, true);
    writer.close()?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()
}

/// `makeDocumentWithFields()`.
fn make_document_with_fields() -> Result<Document> {
    let mut doc = Document::new();
    let stored = FieldType::new();
    stored.set_stored(true)?;
    let indexed_not_tokenized = FieldType::new();
    indexed_not_tokenized.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
    indexed_not_tokenized.set_tokenized(false)?;
    doc.add(Box::new(StringField::new(
        "keyword",
        "test1".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(StringField::new(
        "keyword",
        "test2".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(TextField::new(
        "text",
        "test1".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(TextField::new(
        "text",
        "test2".to_string(),
        Store::YES,
    )?));
    doc.add(Box::new(Field::new(
        "unindexed",
        "test1".to_string(),
        stored.clone(),
    )?));
    doc.add(Box::new(Field::new(
        "unindexed",
        "test2".to_string(),
        stored,
    )?));
    doc.add(Box::new(TextField::new(
        "unstored",
        "test1".to_string(),
        Store::NO,
    )?));
    doc.add(Box::new(TextField::new(
        "unstored",
        "test2".to_string(),
        Store::NO,
    )?));
    doc.add(Box::new(Field::new(
        "indexed_not_tokenized",
        "test1".to_string(),
        indexed_not_tokenized.clone(),
    )?));
    doc.add(Box::new(Field::new(
        "indexed_not_tokenized",
        "test2".to_string(),
        indexed_not_tokenized,
    )?));
    Ok(doc)
}

/// `doAssert(doc, fromIndex)`.
fn do_assert(doc: &Document, from_index: bool) {
    let keyword_field_values = doc.get_fields_by_name("keyword");
    let text_field_values = doc.get_fields_by_name("text");
    let unindexed_field_values = doc.get_fields_by_name("unindexed");
    let unstored_field_values = doc.get_fields_by_name("unstored");

    assert_array_with_size(keyword_field_values.len(), 2);
    assert_array_with_size(text_field_values.len(), 2);
    assert_array_with_size(unindexed_field_values.len(), 2);
    // this test cannot work for documents retrieved from the index
    // since unstored fields will obviously not be returned
    if !from_index {
        assert_array_with_size(unstored_field_values.len(), 2);
    }

    let sv = |f: &dyn IndexableField| f.string_value().unwrap_or_default();
    assert_equal_to(sv(keyword_field_values[0]).as_str(), "test1");
    assert_equal_to(sv(keyword_field_values[1]).as_str(), "test2");
    assert_equal_to(sv(text_field_values[0]).as_str(), "test1");
    assert_equal_to(sv(text_field_values[1]).as_str(), "test2");
    assert_equal_to(sv(unindexed_field_values[0]).as_str(), "test1");
    assert_equal_to(sv(unindexed_field_values[1]).as_str(), "test2");
    // this test cannot work for documents retrieved from the index
    // since unstored fields will obviously not be returned
    if !from_index {
        assert_equal_to(sv(unstored_field_values[0]).as_str(), "test1");
        assert_equal_to(sv(unstored_field_values[1]).as_str(), "test2");
    }
}

/// `field`: the `StringField` "id" the Java test keeps after `doc.add`.
fn id_field(doc: &mut Document) -> &StringField {
    doc.get_field_mut_as::<StringField>("id")
        .expect("INVARIANT: the document holds the StringField `id`")
}

/// Port of `testFieldSetValue`.
fn test_field_set_value() -> Result<()> {
    let field = StringField::new("id", "id1".to_string(), Store::YES)?;
    let mut doc = Document::new();
    doc.add(Box::new(field));
    doc.add(Box::new(StringField::new(
        "keyword",
        "test".to_string(),
        Store::YES,
    )?));

    let mut dir = LuceneTestCase::new_directory();
    let writer = random_writer(&dir)?;
    writer.add_document(&doc)?;
    id_field(&mut doc).set_string_value("id2".to_string())?;
    writer.add_document(&doc)?;
    id_field(&mut doc).set_string_value("id3".to_string())?;
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));

    let query: Arc<dyn Query> = Arc::new(TermQuery::new(Term::from_text("keyword", "test")));

    // ensure that queries return expected results without DateFilter first
    let hits = searcher.search(query, 1000)?.score_docs;
    assert_array_with_size(hits.len(), 3);
    // `Set<String> seen = new HashSet<>()`: only membership is checked.
    let mut seen: Vec<String> = Vec::new();
    let stored_fields = searcher.stored_fields()?;
    for i in 0..3 {
        let doc2 = stored_fields.document(hits[i].doc)?;
        // `Field f = (Field) doc2.getField("id")`.
        let f = doc2.get_field("id").expect("id");
        let v = f.string_value().unwrap_or_default();
        match v.as_str() {
            "id1" | "id2" | "id3" => {
                if !seen.contains(&v) {
                    seen.push(v)
                }
            }
            _ => Assert::fail("unexpected id field"),
        }
    }
    writer.close()?;
    (reader as Arc<dyn IndexReader>).close()?;
    dir.close()?;
    assert_contains_in_any_order("did not see all IDs", &seen, &["id1", "id2", "id3"]);
    Ok(())
}

/// Port of `testInvalidFields` (LUCENE-3616).
fn test_invalid_fields() -> Result<()> {
    expect_throws(IAE, || {
        let mut tok = MockTokenizer::whitespace_lower_case()?;
        tok.set_reader(Box::new(StringCharReader::new("")))?;
        Field::new_with_token_stream(
            "foo",
            Arc::new(AtomicRefCell::new(tok)),
            StringField::type_stored().clone(),
        )
    });
    Ok(())
}

/// Port of `testNumericFieldAsString`.
fn test_numeric_field_as_string() -> Result<()> {
    let mut doc = Document::new();
    doc.add(Box::new(StoredField::new_number(
        "int",
        NumericValue::Int(5),
    )?));
    assert_equal_to(doc.get("int").unwrap_or_default().as_str(), "5");
    assert_null_value(&doc.get("somethingElse"));
    doc.add(Box::new(StoredField::new_number(
        "int",
        NumericValue::Int(4),
    )?));
    assert_array_containing(&doc.get_values("int"), &["5", "4"]);

    let mut dir = LuceneTestCase::new_directory();
    let iw = random_writer(&dir)?;
    iw.add_document(&doc)?;
    let ir = iw.get_reader()?;
    let sdoc = ir.stored_fields()?.document(0)?;
    assert_equal_to(sdoc.get("int").unwrap_or_default().as_str(), "5");
    assert_null_value(&sdoc.get("somethingElse"));
    assert_array_containing(&doc.get_values("int"), &["5", "4"]);
    (ir as Arc<dyn IndexReader>).close()?;
    iw.close()?;
    dir.close()
}
