//! Port of `lucene/core/src/test/org/apache/lucene/index/TestPayloadsOnVectors.java`.
//!
//! `field.setTokenStream(ts)` on a field already added changes the
//! document's own field (`crate::support::field_mut`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::analysis::tokenattributes::PayloadAttribute;
use rucene::analysis::{StringCharReader, TokenStream};
use rucene::document::{Document, Field, FieldType, TextField};
use rucene::error::Result;
use rucene::index::postings_enum::POSTINGS_ENUM_ALL;
use rucene::index::{PostingsEnum, Terms};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::attributes::payload_id;
use rucene_test_framework::analysis::canned_token_stream::CannedTokenStream;
use rucene_test_framework::analysis::mock_tokenizer::{MockTokenizer, WHITESPACE};
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, IAE};
use crate::support::{field_mut, npe};

/// `TestPayloadsOnVectors`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestPayloadsOnVectors",
        vec![
            test("testMixupDocs", test_mixup_docs),
            test("testMixupMultiValued", test_mixup_multi_valued),
            test(
                "testPayloadsWithoutPositions",
                test_payloads_without_positions,
            ),
        ],
    )
}

/// some docs have payload att, some not
///
/// `new FieldType(TextField.TYPE_NOT_STORED)` with term vectors, the given
/// positions, payloads, and random offsets.
fn vector_type(positions: bool) -> Result<Arc<FieldType>> {
    let custom_type = FieldType::new_from(TextField::type_not_stored());
    custom_type.set_store_term_vectors(true)?;
    custom_type.set_store_term_vector_positions(positions)?;
    custom_type.set_store_term_vector_payloads(true)?;
    custom_type.set_store_term_vector_offsets(random().next_boolean())?;
    Ok(Arc::new(custom_type))
}

/// `new MockTokenizer(MockTokenizer.WHITESPACE, true)` with
/// `setReader(new StringReader(text))`, as a token stream value.
fn whitespace_stream(text: &str) -> Result<Arc<AtomicRefCell<dyn TokenStream>>> {
    let mut ts = MockTokenizer::new(Arc::clone(&WHITESPACE), true)?;
    ts.set_reader(Box::new(StringCharReader::new(text)))?;
    Ok(Arc::new(AtomicRefCell::new(ts)))
}

/// `new CannedTokenStream(withPayload)`, `withPayload` being `new
/// Token("withPayload", 0, 11)` with the payload `"test"`; checks
/// `ts.hasAttribute(PayloadAttribute.class)`.
fn with_payload_stream() -> Arc<AtomicRefCell<dyn TokenStream>> {
    let mut with_payload = Token::with_offsets("withPayload", 0, 11);
    with_payload.set_payload(Some(BytesRef::from_char_sequence("test")));
    let ts = CannedTokenStream::new(vec![with_payload]);
    Assert::assert_true(ts.attribute_source().has_attribute_by_id(payload_id()));
    Arc::new(AtomicRefCell::new(ts))
}

/// The term vector checks both mixup tests make: `withPayload` at
/// `position`, with the payload `"test"`.
fn check_with_payload(terms: Option<Box<dyn Terms>>, position: i32) -> Result<()> {
    debug_assert!(terms.is_some());
    let terms = terms.unwrap_or_else(|| npe("terms"));
    let mut terms_enum = terms.iterator()?;
    Assert::assert_true(terms_enum.seek_exact(&BytesRef::from_char_sequence("withPayload"))?);
    let mut de: Box<dyn PostingsEnum> = terms_enum
        .postings(None, POSTINGS_ENUM_ALL)?
        .ok_or_else(rucene::error::LuceneError::null_pointer)?;
    assert_equals(&0, &de.next_doc()?);
    assert_equals(&position, &de.next_position()?);
    assert_equals(
        &Some(BytesRef::from_char_sequence("test")),
        &de.get_payload()?.map(|p| BytesRef::new(p.to_vec())),
    );
    Ok(())
}

/// Port of `TestPayloadsOnVectors.testMixupDocs`: some docs have payload
/// att, some not.
fn test_mixup_docs() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(MockAnalyzer::new(
        &*random(),
    ))));
    iwc.set_merge_policy(Some(Arc::new(LuceneTestCase::new_log_merge_policy())))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let mut doc = Document::new();
    let custom_type = vector_type(true)?;
    let ts = whitespace_stream("here we go")?;
    let field = Field::new_with_token_stream("field", ts, custom_type)?;
    doc.add(Box::new(field));
    writer.add_document(&doc)?;

    let ts = with_payload_stream();
    field_mut(&mut doc, "field").set_token_stream_value(ts)?;
    writer.add_document(&doc)?;

    let ts = whitespace_stream("another")?;
    field_mut(&mut doc, "field").set_token_stream_value(ts)?;
    writer.add_document(&doc)?;

    let reader = writer.get_reader()?;
    let terms = reader.term_vectors()?.get_field(1, "field")?;
    check_with_payload(terms, 0)?;
    writer.close()?;
    reader.close()?;
    dir.close()
}

/// Port of `TestPayloadsOnVectors.testMixupMultiValued`: some field
/// instances have payload att, some not.
fn test_mixup_multi_valued() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type = vector_type(true)?;
    let ts = whitespace_stream("here we go")?;
    let field = Field::new_with_token_stream("field", ts, Arc::clone(&custom_type))?;
    doc.add(Box::new(field));
    let ts = with_payload_stream();
    let field2 = Field::new_with_token_stream("field", ts, Arc::clone(&custom_type))?;
    doc.add(Box::new(field2));
    let ts = whitespace_stream("nopayload")?;
    let field3 = Field::new_with_token_stream("field", ts, Arc::clone(&custom_type))?;
    doc.add(Box::new(field3));
    writer.add_document(&doc)?;
    let reader = writer.get_reader()?;
    let terms = reader.term_vectors()?.get_field(0, "field")?;
    check_with_payload(terms, 3)?;
    writer.close()?;
    reader.close()?;
    dir.close()
}

/// Port of `TestPayloadsOnVectors.testPayloadsWithoutPositions`.
fn test_payloads_without_positions() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let writer = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut doc = Document::new();
    let custom_type = vector_type(false)?;
    doc.add(Box::new(Field::new(
        "field",
        "foo".to_string(),
        custom_type,
    )?));

    expect_throws(IAE, || writer.add_document(&doc));

    writer.close()?;
    dir.close()
}
