//! Port of `lucene/core/src/test/org/apache/lucene/index/TestReaderClosed.java`.

use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{Class, Constructor, Method};
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::{DirectoryReader, IndexReader, LeafReader, ParallelLeafReader};
use rucene::search::term_range_query::TermRangeQuery;
use rucene::search::Query;
use rucene_test_framework::analysis::mock_tokenizer::KEYWORD;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::{OwnCacheKeyMultiReader, RandomIndexWriter};
use rucene_test_framework::util::java_exceptions::{is_instance, java_throwable, rethrow_java};
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, random};
use crate::support::field_mut;

/// The instance fields of `TestReaderClosed`.
#[derive(Default)]
struct TestReaderClosedInstance {
    /// `private DirectoryReader reader`.
    reader: Option<Arc<dyn DirectoryReader>>,
    /// `private Directory dir`.
    dir: Option<TestDirectory>,
}

impl TestReaderClosedInstance {
    fn reader(&self) -> &Arc<dyn DirectoryReader> {
        self.reader
            .as_ref()
            .unwrap_or_else(|| crate::support::npe("reader"))
    }
}

/// A test method over the instance.
fn instance_test(name: &str, body: fn(&mut TestReaderClosedInstance) -> Result<()>) -> Method {
    Method::new(name).throws(&["java.lang.Exception"]).body(
        move |inst: &mut TestReaderClosedInstance| body(inst).unwrap_or_else(|e| rethrow_java(e)),
    )
}

/// `TestReaderClosed`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestReaderClosed")
        .extends(&LuceneTestCase::class())
        .constructor(Constructor::no_args(TestReaderClosedInstance::default))
        .method(instance_test("setUp", set_up))
        .method(instance_test("tearDown", tear_down))
        .method(instance_test("test", test))
        .method(instance_test("testReaderChaining", test_reader_chaining))
        .build()
}

/// `public void setUp()`.
fn set_up(inst: &mut TestReaderClosedInstance) -> Result<()> {
    LuceneTestCase::super_set_up();
    let dir = LuceneTestCase::new_directory();
    let conf = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
        MockAnalyzer::with_automaton(&*random(), Arc::clone(&KEYWORD), false),
    )));
    conf.set_max_buffered_docs(TestUtil::next_int(&*random(), 50, 1000))?;
    let writer = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), conf)?;

    let mut doc = Document::new();
    let field = LuceneTestCase::new_string_field("field", "", Store::NO);
    doc.add(Box::new(field));

    // we generate aweful prefixes: good for testing.
    // but for preflex codec, the test can be very slow, so use less iterations.
    let num = LuceneTestCase::at_least(10);
    for _i in 0..num {
        field_mut(&mut doc, "field")
            .set_string_value(TestUtil::random_unicode_string_max(&*random(), 10))?;
        writer.add_document(&doc)?;
    }
    writer.force_merge(1)?;
    inst.reader = Some(writer.get_reader()?);
    writer.close()?;
    inst.dir = Some(dir);
    Ok(())
}

/// `TermRangeQuery.newStringRange("field", "a", "z", true, true)`.
fn range_query() -> Result<Arc<dyn Query>> {
    Ok(Arc::new(TermRangeQuery::new_string_range(
        "field",
        Some("a"),
        Some("z"),
        true,
        true,
    )?))
}

/// Port of `TestReaderClosed.test`.
fn test(inst: &mut TestReaderClosedInstance) -> Result<()> {
    let reader = Arc::clone(inst.reader());
    Assert::assert_true(reader.get_ref_count() > 0);
    let searcher = LuceneTestCase::new_searcher(Arc::clone(&reader));
    let query = range_query()?;
    searcher.search(Arc::clone(&query), 5)?;
    reader.close()?;
    match searcher.search(query, 5) {
        Ok(_) => {}
        // expected
        Err(e) if is_instance(&e, "org.apache.lucene.store.AlreadyClosedException") => {}
        // expected if the searcher has been created with threads since LuceneTestCase
        // closes the thread-pool in a reader close listener
        Err(e) if is_instance(&e, "java.util.concurrent.RejectedExecutionException") => {}
        Err(e) => return Err(e),
    }
    Ok(())
}

/// LUCENE-3800.
///
/// Port of `TestReaderClosed.testReaderChaining`.
fn test_reader_chaining(inst: &mut TestReaderClosedInstance) -> Result<()> {
    let reader = Arc::clone(inst.reader());
    Assert::assert_true(reader.get_ref_count() > 0);
    let wrapped_reader: Arc<dyn LeafReader> = Arc::new(ParallelLeafReader::new(vec![
        LuceneTestCase::get_only_leaf_reader(Arc::clone(&reader) as Arc<dyn IndexReader>)?,
    ])?);

    // We wrap with a OwnCacheKeyMultiReader so that closing the underlying reader
    // does not terminate the threadpool (if that index searcher uses one)
    let multi: Arc<dyn rucene::index::CompositeReader> =
        Arc::new(OwnCacheKeyMultiReader::new(vec![
            wrapped_reader.as_index_reader()
        ])?);
    let searcher = LuceneTestCase::new_searcher(multi);

    let query = range_query()?;
    let result = (|| -> Result<()> {
        searcher.search(Arc::clone(&query), 5)?;
        reader.close()?; // close original child reader
        if let Err(e) = searcher.search(Arc::clone(&query), 5) {
            let mut ace = None;
            let mut t = Some(java_throwable(&e));
            while let Some(cur) = t {
                if cur.is_instance("org.apache.lucene.store.AlreadyClosedException") {
                    ace = Some(cur.clone());
                }
                t = cur.cause().cloned();
            }
            match ace {
                None => Throwable::with_cause(
                    "java.lang.AssertionError",
                    Some("Query failed, but not due to an AlreadyClosedException".to_string()),
                    java_throwable(&e),
                )
                .throw(),
                Some(ace) => assert_equals(
                    &Some(
                        "this IndexReader cannot be used anymore as one of its child readers was closed",
                    ),
                    &ace.message(),
                ),
            }
        }
        Ok(())
    })();
    // close executor: in case of wrap-wrap-wrapping
    let closed = searcher.get_index_reader().close();
    result?;
    closed
}

/// `public void tearDown()`.
fn tear_down(inst: &mut TestReaderClosedInstance) -> Result<()> {
    if let Some(dir) = inst.dir.take() {
        dir.close()?;
    }
    LuceneTestCase::super_tear_down();
    Ok(())
}
