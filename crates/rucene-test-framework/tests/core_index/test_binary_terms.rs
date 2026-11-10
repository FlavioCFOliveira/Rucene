//! Port of `lucene/core/src/test/org/apache/lucene/index/TestBinaryTerms.java`:
//! Test indexing and searching some byte[] terms.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{Document, FieldType, Store};
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene::search::TermQuery;
use rucene::util::BytesRef;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test};

/// `TestBinaryTerms`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestBinaryTerms",
        vec![
            test("testBinary", test_binary),
            test("testToString", test_to_string),
        ],
    )
}

/// Port of `TestBinaryTerms.testBinary`.
fn test_binary() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = RandomIndexWriter::new_with_mock_analyzer(&*random(), dir.as_directory())?;
    let mut bytes = BytesRef::new(vec![0u8; 2]);
    bytes.length = 0;

    for i in 0..256 {
        bytes.bytes[0] = i as u8;
        bytes.bytes[1] = (255 - i) as u8;
        bytes.length = 2;
        let mut doc = Document::new();
        let mut custom_type = FieldType::new();
        custom_type.set_stored(true)?;
        doc.add(Box::new(LuceneTestCase::new_field(
            "id",
            &i.to_string(),
            &custom_type,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field_bytes(
            "bytes",
            bytes.clone(),
            Store::NO,
        )));
        iw.add_document(&doc)?;
    }

    let ir = iw.get_reader()?;
    iw.close()?;

    let is = LuceneTestCase::new_searcher(Arc::clone(&ir));

    for i in 0..256 {
        bytes.bytes[0] = i as u8;
        bytes.bytes[1] = (255 - i) as u8;
        bytes.length = 2;
        let docs = is.search(
            Arc::new(TermQuery::new(Term::new("bytes", bytes.clone()))),
            5,
        )?;
        assert_equals(&1, &docs.total_hits.value());
        assert_equals(
            &Some(i.to_string()),
            &is.stored_fields()?
                .document(docs.score_docs[0].doc)?
                .get("id"),
        );
    }

    ir.close()?;
    dir.close()
}

/// Port of `TestBinaryTerms.testToString`.
fn test_to_string() -> Result<()> {
    let term = Term::new("foo", BytesRef::new(vec![0xff, 0xfe]));
    assert_equals(&"foo:[ff fe]", &term.to_string().as_str());
    Ok(())
}
