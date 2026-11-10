//! Port of `org.apache.lucene.index.TestFieldReuse` (`extends
//! BaseTokenStreamTestCase`): "test tokenstream reuse by
//! DefaultIndexingChain".
//!
//! `assertSame`/`assertNotSame` compare the identity (address) of the token
//! streams.
//!
//! **Divergence (declared).** Java passes a `null` analyzer to
//! `Field.tokenStream(null, reuse)` and to `new IndexWriterConfig(null)`;
//! the port's `IndexableField::token_stream` and `IndexWriterConfig` take an
//! analyzer, so a `MockAnalyzer` and the default config stand for them. No
//! analyzer is used: the fields are not tokenized or produce their own
//! stream.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{
    Document, Field, InvertableType, NumericValue, SharedReader, StoredValue, StringField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexWriterConfig, IndexableField, IndexableFieldType};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{BaseTokenStreamTestCase, CannedTokenStream, MockAnalyzer};
use rucene_test_framework::util::LuceneTestCase;

use crate::lucene_core_shims::t;

/// The identity of a token stream: its address.
fn identity(ts: &dyn TokenStream) -> *const () {
    ts as *const dyn TokenStream as *const ()
}

/// `assertSame(expected, actual)` over two identities.
fn assert_same(expected: *const (), actual: *const ()) {
    if expected != actual {
        Assert::fail(&format!(
            "expected same:<{expected:p}> was not:<{actual:p}>"
        ));
    }
}

/// `assertNotSame(unexpected, actual)` over two identities.
fn assert_not_same(unexpected: *const (), actual: *const ()) {
    if unexpected == actual {
        Assert::fail(&format!("expected not same:<{unexpected:p}>"));
    }
}

/// The analyzer standing for Java's `null` (see the module docs).
fn null_analyzer() -> MockAnalyzer {
    MockAnalyzer::new(&*LuceneTestCase::random())
}

fn test_string_field() -> Result<()> {
    let analyzer = null_analyzer();
    let mut string_field = Field::new(
        "foo",
        "bar".to_string(),
        StringField::type_not_stored().clone(),
    )?;

    // passing null
    let mut ts = string_field.token_stream(&analyzer, None);
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets(&mut *ts, &["bar"], &[0], &[3])?;

    // now reuse previous stream
    string_field = Field::new(
        "foo",
        "baz".to_string(),
        StringField::type_not_stored().clone(),
    )?;
    let ts_identity = identity(&*ts);
    let ts2 = string_field.token_stream(&analyzer, Some(&mut *ts));
    assert_same(ts_identity, identity(&*ts2));
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets(&mut *ts, &["baz"], &[0], &[3])?;

    // pass a bogus stream and ensure it's still ok
    string_field = Field::new(
        "foo",
        "beer".to_string(),
        StringField::type_not_stored().clone(),
    )?;
    let mut bogus = CannedTokenStream::new(Vec::new());
    let bogus_identity = identity(&bogus);
    ts = string_field.token_stream(&analyzer, Some(&mut bogus));
    assert_not_same(identity(&*ts), bogus_identity);
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets(&mut *ts, &["beer"], &[0], &[4])?;
    Ok(())
}

/// The fields `lastSeen` and `lastReturned` of `MyField`, as identities
/// (`None` is `null`), shared with the test.
#[derive(Default)]
struct MyFieldState {
    last_seen: Cell<Option<*const ()>>,
    last_returned: Cell<Option<*const ()>>,
}

/// `MyField implements IndexableField` (`static class`).
struct MyField {
    state: Rc<MyFieldState>,
}

impl std::fmt::Display for MyField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `Object.toString()`.
        write!(
            f,
            "org.apache.lucene.index.TestFieldReuse$MyField@{:p}",
            self
        )
    }
}

impl IndexableField for MyField {
    fn name(&self) -> &str {
        "foo"
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        StringField::type_not_stored()
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        self.state
            .last_seen
            .set(reuse.map(|reuse| identity(&*reuse)));
        let returned: Box<dyn TokenStream> =
            Box::new(CannedTokenStream::new(vec![Token::with_offsets(
                "unimportant",
                0,
                10,
            )]));
        self.state.last_returned.set(Some(identity(&*returned)));
        returned
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::TOKEN_STREAM)
    }
}

fn test_index_writer_actually_reuses() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let iwc = IndexWriterConfig::new();
    let iw = IndexWriter::new(dir.as_directory(), iwc)?;
    let field1 = Rc::new(MyFieldState::default());
    // `iw.addDocument(Collections.singletonList(field1))`
    let mut doc1 = Document::new();
    doc1.add(Box::new(MyField {
        state: Rc::clone(&field1),
    }));
    iw.add_document(&doc1)?;
    let previous = field1.last_returned.get();
    Assert::assert_true(previous.is_some());

    let field2 = Rc::new(MyFieldState::default());
    let mut doc2 = Document::new();
    doc2.add(Box::new(MyField {
        state: Rc::clone(&field2),
    }));
    iw.add_document(&doc2)?;
    match (previous, field2.last_seen.get()) {
        (Some(previous), Some(last_seen)) => assert_same(previous, last_seen),
        (_, _) => Assert::fail("expected same: the previous stream, was: <null>"),
    }
    iw.close()?;
    drop(iw);
    dir.close()
}

/// The class `org.apache.lucene.index.TestFieldReuse`.
pub fn class() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestFieldReuse")
        .extends(&BaseTokenStreamTestCase::class())
        .constructor(Constructor::no_args(|| ()))
        .method(t("testStringField", test_string_field))
        .method(t(
            "testIndexWriterActuallyReuses",
            test_index_writer_actually_reuses,
        ))
        .build()
}
