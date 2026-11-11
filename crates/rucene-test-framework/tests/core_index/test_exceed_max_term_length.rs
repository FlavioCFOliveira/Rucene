//! Port of `lucene/core/src/test/org/apache/lucene/index/TestExceedMaxTermLength.java`.
//!
//! Tests that a useful exception is thrown when attempting to index a term
//! that is too large
//!
//! @see IndexWriter#MAX_TERM_LENGTH

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::{After, Assert, Before};
use rucene::document::{Document, Field, FieldType};
use rucene::error::Result;
use rucene::index::index_writer::{IndexWriter, MAX_TERM_LENGTH};
use rucene::index::IndexOptions;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::{expect_throws, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{random, IAE};

const MIN_TEST_TERM_LENGTH: i32 = MAX_TERM_LENGTH as i32 + 1;
const MAX_TEST_TERM_LENGTH: i32 = MAX_TERM_LENGTH as i32 * 2;

/// The `dir` field of `TestExceedMaxTermLength`.
#[derive(Default)]
struct Instance {
    dir: Option<TestDirectory>,
}

impl Instance {
    fn dir(&self) -> Arc<dyn rucene::store::Directory> {
        self.dir
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("dir"))
            .as_directory()
    }
}

fn t(name: &str, body: fn(&Instance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// `TestExceedMaxTermLength`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestExceedMaxTermLength")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(
            Method::new("createDir")
                .annotate(Before)
                .body(|inst: &mut Instance| inst.dir = Some(LuceneTestCase::new_directory())),
        )
        .method(
            Method::new("destroyDir")
                .annotate(After)
                .throws(&["java.io.IOException"])
                .body(|inst: &mut Instance| {
                    if let Some(dir) = inst.dir.take() {
                        dir.close().unwrap_or_else(|e| rethrow_java(e));
                    } else {
                        crate::support::npe::<()>("dir");
                    }
                }),
        )
        .method(t("testTokenStream", test_token_stream))
        .method(t("testBinaryValue", test_binary_value))
        .build()
}

/// The four `assertTrue` checks both tests make on the exception message.
fn check_message(msg: &str, name: &str) {
    let max_length_msg = MAX_TERM_LENGTH.to_string();
    Assert::assert_true_msg(
        &format!("IllegalArgumentException didn't mention 'immense term': {msg}"),
        msg.contains("immense term"),
    );
    Assert::assert_true_msg(
        &format!("IllegalArgumentException didn't mention max length ({max_length_msg}): {msg}"),
        msg.contains(&max_length_msg),
    );
    Assert::assert_true_msg(
        &format!("IllegalArgumentException didn't mention field name ({name}): {msg}"),
        msg.contains(name),
    );
    Assert::assert_true_msg(
        &format!("IllegalArgumentException didn't mention original message: {msg}"),
        msg.contains("bytes can be at most") && msg.contains("in length; got"),
    );
}

/// Port of `TestExceedMaxTermLength.testTokenStream`.
fn test_token_stream(inst: &Instance) -> Result<()> {
    let mock_analyzer = MockAnalyzer::new(&*random());
    mock_analyzer.set_max_token_length(i32::MAX);
    let w = IndexWriter::new(
        inst.dir(),
        LuceneTestCase::new_index_writer_config_with(&*random(), Some(Arc::new(mock_analyzer))),
    )?;
    let result = (|| -> Result<()> {
        let ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)?;
        ft.set_stored(random().next_boolean())?;
        ft.freeze();

        let mut doc = Document::new();
        if random().next_boolean() {
            // totally ok short field value
            doc.add(Box::new(Field::new(
                &TestUtil::random_simple_string_between(&*random(), 1, 10),
                TestUtil::random_simple_string_between(&*random(), 1, 10),
                ft.clone(),
            )?));
        }
        // problematic field
        let name = TestUtil::random_simple_string_between(&*random(), 1, 50);
        let value = TestUtil::random_simple_string_between(
            &*random(),
            MIN_TEST_TERM_LENGTH,
            MAX_TEST_TERM_LENGTH,
        );
        let f = Field::new(&name, value, ft.clone())?;
        if random().next_boolean() {
            // totally ok short field value
            doc.add(Box::new(Field::new(
                &TestUtil::random_simple_string_between(&*random(), 1, 10),
                TestUtil::random_simple_string_between(&*random(), 1, 10),
                ft.clone(),
            )?));
        }
        doc.add(Box::new(f));

        let expected = expect_throws(IAE, || w.add_document(&doc));
        let msg = expected.message().unwrap_or_default().to_string();
        check_message(&msg, &name);
        Ok(())
    })();
    let closed = w.close();
    result?;
    closed
}

/// Port of `TestExceedMaxTermLength.testBinaryValue`.
fn test_binary_value(inst: &Instance) -> Result<()> {
    let w = IndexWriter::new(inst.dir(), LuceneTestCase::new_index_writer_config())?;
    let result = (|| -> Result<()> {
        let ft = FieldType::new();
        ft.set_index_options(*RandomPicks::random_from(
            &*random(),
            &[IndexOptions::DOCS, IndexOptions::DOCS_AND_FREQS],
        ))?;
        ft.set_stored(random().next_boolean())?;
        ft.set_tokenized(false)?;
        ft.freeze();

        let mut doc = Document::new();
        if random().next_boolean() {
            // totally ok short field value
            doc.add(Box::new(Field::new_with_bytes(
                &TestUtil::random_simple_string_between(&*random(), 1, 10),
                TestUtil::random_binary_term_with(&*random(), 10),
                ft.clone(),
            )?));
        }
        // problematic field
        let name = TestUtil::random_simple_string_between(&*random(), 1, 50);
        let value = TestUtil::random_binary_term_with(
            &*random(),
            TestUtil::next_int(&*random(), MIN_TEST_TERM_LENGTH, MAX_TEST_TERM_LENGTH),
        );
        let f = Field::new_with_bytes(&name, value, ft.clone())?;
        if random().next_boolean() {
            // totally ok short field value
            doc.add(Box::new(Field::new_with_bytes(
                &TestUtil::random_simple_string_between(&*random(), 1, 10),
                TestUtil::random_binary_term_with(&*random(), 10),
                ft.clone(),
            )?));
        }
        doc.add(Box::new(f));

        let expected = expect_throws(IAE, || w.add_document(&doc));
        let msg = expected.message().unwrap_or_default().to_string();
        check_message(&msg, &name);
        Ok(())
    })();
    let closed = w.close();
    result?;
    closed
}
