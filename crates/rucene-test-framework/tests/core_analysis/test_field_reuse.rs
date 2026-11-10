//! Port of `org.apache.lucene.index.TestFieldReuse` (`extends
//! BaseTokenStreamTestCase`): "test tokenstream reuse by
//! DefaultIndexingChain".
//!
//! `assertSame`/`assertNotSame` compare the identity (address) of the token
//! streams.

use std::sync::Arc;
use std::sync::Mutex;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::analysis::{Analyzer, TokenStream};
use rucene::document::{
    Field, InvertableType, NumericValue, SharedReader, StoredValue, StringField,
};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::{IndexWriterConfig, IndexableField, IndexableFieldType};
use rucene::util::BytesRef;
use rucene_test_framework::analysis::token::Token;
use rucene_test_framework::analysis::{BaseTokenStreamTestCase, CannedTokenStream};
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

fn test_string_field() -> Result<()> {
    let mut string_field = Field::new(
        "foo",
        "bar".to_string(),
        StringField::type_not_stored().clone(),
    )?;

    // passing null
    let mut ts = string_field.token_stream(None, None)?;
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets(&mut *ts, &["bar"], &[0], &[3])?;

    // now reuse previous stream
    string_field = Field::new(
        "foo",
        "baz".to_string(),
        StringField::type_not_stored().clone(),
    )?;
    let ts_identity = identity(&*ts);
    // `ts` moves into the call, as Java hands the object over; `ts2` is that
    // object when it is reused (`assertSame(ts, ts2)`).
    ts = string_field.token_stream(None, Some(ts))?;
    assert_same(ts_identity, identity(&*ts));
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets(&mut *ts, &["baz"], &[0], &[3])?;

    // pass a bogus stream and ensure it's still ok
    string_field = Field::new(
        "foo",
        "beer".to_string(),
        StringField::type_not_stored().clone(),
    )?;
    let bogus: Box<dyn TokenStream> = Box::new(CannedTokenStream::new(Vec::new()));
    let bogus_identity = identity(&*bogus);
    ts = string_field.token_stream(None, Some(bogus))?;
    assert_not_same(identity(&*ts), bogus_identity);
    BaseTokenStreamTestCase::assert_token_stream_contents_offsets(&mut *ts, &["beer"], &[0], &[4])?;
    Ok(())
}

/// The fields `lastSeen` and `lastReturned` of `MyField`, as identities
/// (addresses; `None` is `null`), shared with the test.
#[derive(Default)]
struct MyFieldState {
    last_seen: Mutex<Option<usize>>,
    last_returned: Mutex<Option<usize>>,
}

/// `MyField implements IndexableField` (`static class`).
struct MyField {
    state: Arc<MyFieldState>,
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
        _analyzer: Option<&dyn Analyzer>,
        reuse: Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        *self.state.last_seen.lock().unwrap() = reuse.map(|reuse| identity(&*reuse) as usize);
        let returned: Box<dyn TokenStream> =
            Box::new(CannedTokenStream::new(vec![Token::with_offsets(
                "unimportant",
                0,
                10,
            )]));
        *self.state.last_returned.lock().unwrap() = Some(identity(&*returned) as usize);
        Ok(returned)
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
    let iwc = IndexWriterConfig::with_analyzer(None);
    let iw = IndexWriter::new(dir.as_directory(), iwc)?;
    let field1 = Arc::new(MyFieldState::default());
    let doc1: Vec<Box<dyn IndexableField>> = vec![Box::new(MyField {
        state: Arc::clone(&field1),
    })];
    iw.add_document(&doc1)?;
    let previous = *field1.last_returned.lock().unwrap();
    Assert::assert_true(previous.is_some());

    let field2 = Arc::new(MyFieldState::default());
    let doc2: Vec<Box<dyn IndexableField>> = vec![Box::new(MyField {
        state: Arc::clone(&field2),
    })];
    iw.add_document(&doc2)?;
    let last_seen = *field2.last_seen.lock().unwrap();
    match (previous, last_seen) {
        (Some(previous), Some(last_seen)) => {
            assert_same(previous as *const (), last_seen as *const ())
        }
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
