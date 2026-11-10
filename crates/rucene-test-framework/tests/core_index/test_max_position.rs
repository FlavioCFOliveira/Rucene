//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMaxPosition.java`.
//!
//! LUCENE-6382

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::analysis::tokenattributes::{PayloadAttribute, PositionIncrementAttribute};
use rucene::analysis::TokenStream;
use rucene::document::{Document, TextField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, MAX_POSITION};
use rucene::index::multi_fields::MultiTerms;
use rucene::index::{IndexReader, POSTINGS_ENUM_ALL};
use rucene::util::atomic_ref_cell::AtomicRefCell;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::{CannedTokenStream, Token};
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, random, test, IAE};
use crate::support::npe;

/// `TestMaxPosition`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMaxPosition",
        vec![
            test("testTooBigPosition", test_too_big_position),
            test("testMaxPosition", test_max_position),
        ],
    )
}

/// `new TextField("foo", new CannedTokenStream(new Token[] {t1, t2}))`.
fn canned_text_field(tokens: Vec<Token>) -> Result<TextField> {
    let stream: Arc<AtomicRefCell<dyn TokenStream>> =
        Arc::new(AtomicRefCell::new(CannedTokenStream::new(tokens)));
    TextField::new_with_token_stream("foo", stream)
}

/// Port of `TestMaxPosition.testTooBigPosition`.
fn test_too_big_position() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let mut doc = Document::new();
    // This is at position 1:
    let mut t1 = Token::with_offsets("foo", 0, 3);
    t1.set_position_increment(2);
    if random().next_boolean() {
        t1.set_payload(Some(BytesRef::new(vec![0x1])));
    }
    let mut t2 = Token::with_offsets("foo", 4, 7);
    // This should overflow max:
    t2.set_position_increment(MAX_POSITION);
    if random().next_boolean() {
        t2.set_payload(Some(BytesRef::new(vec![0x1])));
    }
    doc.add(Box::new(canned_text_field(vec![t1, t2])?));
    expect_throws(IAE, || iw.add_document(&doc));

    // Document should not be visible:
    let r = directory_reader::open_with_writer(&*iw)?;
    assert_equals(&0, &r.num_docs());
    r.close()?;
    iw.close()?;
    drop(iw);
    dir.close()
}

/// Port of `TestMaxPosition.testMaxPosition`.
fn test_max_position() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iw = IndexWriter::new(
        dir.as_directory(),
        LuceneTestCase::new_index_writer_config_analyzer(None),
    )?;
    let mut doc = Document::new();
    // This is at position 0:
    let mut t1 = Token::with_offsets("foo", 0, 3);
    if random().next_boolean() {
        t1.set_payload(Some(BytesRef::new(vec![0x1])));
    }
    let mut t2 = Token::with_offsets("foo", 4, 7);
    t2.set_position_increment(MAX_POSITION);
    if random().next_boolean() {
        t2.set_payload(Some(BytesRef::new(vec![0x1])));
    }
    doc.add(Box::new(canned_text_field(vec![t1, t2])?));
    iw.add_document(&doc)?;

    // Document should be visible:
    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*iw)?;
    assert_equals(&1, &r.num_docs());
    let mut postings = MultiTerms::get_term_postings_enum(
        &r,
        "foo",
        &BytesRef::from_char_sequence("foo"),
        POSTINGS_ENUM_ALL,
    )?
    .unwrap_or_else(|| npe("postings"));

    // "foo" appears in docID=0
    assert_equals(&0, &postings.next_doc()?);

    // "foo" appears 2 times in the doc
    assert_equals(&2, &postings.freq()?);

    // first at pos=0
    assert_equals(&0, &postings.next_position()?);

    // next at pos=MAX
    assert_equals(&MAX_POSITION, &postings.next_position()?);

    r.close()?;
    iw.close()?;
    drop(iw);
    dir.close()
}
