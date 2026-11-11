//! Port of `lucene/core/src/test/org/apache/lucene/index/TestIsCurrent.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::terms::Term;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals_msg, random};

/// The `writer` and `directory` fields of `TestIsCurrent`.
#[derive(Default)]
struct Instance {
    writer: Option<RandomIndexWriter>,
    directory: Option<TestDirectory>,
}

impl Instance {
    fn writer(&self) -> &RandomIndexWriter {
        self.writer
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("writer"))
    }
}

/// A test method of the instance.
fn ti(name: &str, body: fn(&Instance) -> Result<()>) -> Method {
    Method::new(name)
        .throws(&["java.io.IOException"])
        .body(move |inst: &mut Instance| body(inst).unwrap_or_else(|e| rethrow_java(e)))
}

/// `TestIsCurrent`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestIsCurrent")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(Instance::default))
        .method(Method::new("setUp").throws(&["java.lang.Exception"]).body(
            |inst: &mut Instance| {
                LuceneTestCase::super_set_up();
                set_up(inst).unwrap_or_else(|e| rethrow_java(e));
            },
        ))
        .method(
            Method::new("tearDown")
                .throws(&["java.lang.Exception"])
                .body(|inst: &mut Instance| {
                    LuceneTestCase::super_tear_down();
                    tear_down(inst).unwrap_or_else(|e| rethrow_java(e));
                }),
        )
        .method(ti(
            "testDeleteByTermIsCurrent",
            test_delete_by_term_is_current,
        ))
        .method(ti("testDeleteAllIsCurrent", test_delete_all_is_current))
        .build()
}

/// The body of `setUp()` after `super.setUp()`.
fn set_up(inst: &mut Instance) -> Result<()> {
    // initialize directory
    let directory = inst.directory.insert(LuceneTestCase::new_directory());
    let writer = inst
        .writer
        .insert(RandomIndexWriter::new_with_mock_analyzer(
            &*random(),
            directory.as_directory(),
        )?);

    // write document
    let mut doc = Document::new();
    doc.add(Box::new(LuceneTestCase::new_text_field(
        "UUID",
        "1",
        Store::YES,
    )));
    writer.add_document(&doc)?;
    writer.commit()?;
    Ok(())
}

/// The body of `tearDown()` after `super.tearDown()`.
fn tear_down(inst: &mut Instance) -> Result<()> {
    match inst.writer.take() {
        Some(writer) => writer.close()?,
        None => crate::support::npe("writer"),
    }
    match inst.directory.take() {
        Some(directory) => directory.close(),
        None => crate::support::npe("directory"),
    }
}

/// Failing testcase showing the trouble
///
/// Port of `TestIsCurrent.testDeleteByTermIsCurrent`.
fn test_delete_by_term_is_current(inst: &Instance) -> Result<()> {
    let writer = inst.writer();
    // get reader
    let reader = writer.get_reader()?;

    // assert index has a document and reader is up2date
    assert_equals_msg(
        Some("One document should be in the index"),
        &1,
        &writer.get_doc_stats()?.num_docs,
    );
    Assert::assert_true_msg(
        "One document added, reader should be current",
        reader.is_current()?,
    );

    // remove document
    let id_term = Term::from_text("UUID", "1");
    writer.delete_documents(id_term)?;
    writer.commit()?;

    // assert document has been deleted (index changed), reader is stale
    assert_equals_msg(
        Some("Document should be removed"),
        &0,
        &writer.get_doc_stats()?.num_docs,
    );
    Assert::assert_false_msg("Reader should be stale", reader.is_current()?);

    reader.close()
}

/// Testcase for example to show that writer.deleteAll() is working as expected
///
/// Port of `TestIsCurrent.testDeleteAllIsCurrent`.
fn test_delete_all_is_current(inst: &Instance) -> Result<()> {
    let writer = inst.writer();
    // get reader
    let reader = writer.get_reader()?;

    // assert index has a document and reader is up2date
    assert_equals_msg(
        Some("One document should be in the index"),
        &1,
        &writer.get_doc_stats()?.num_docs,
    );
    Assert::assert_true_msg(
        "Document added, reader should be stale ",
        reader.is_current()?,
    );

    // remove all documents
    writer.delete_all()?;
    writer.commit()?;

    // assert document has been deleted (index changed), reader is stale
    assert_equals_msg(
        Some("Document should be removed"),
        &0,
        &writer.get_doc_stats()?.num_docs,
    );
    Assert::assert_false_msg("Reader should be stale", reader.is_current()?);

    reader.close()
}
