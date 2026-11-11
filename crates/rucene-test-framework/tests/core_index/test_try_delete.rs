//! Port of `lucene/core/src/test/org/apache/lucene/index/TestTryDelete.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, StringField};
use rucene::error::Result;
use rucene::index::directory_reader;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::{IndexWriterConfig, OpenMode};
use rucene::index::merge_policy::LogMergePolicy;
use rucene::index::terms::Term;
use rucene::index::IndexReader;
use rucene::search::searcher_factory::DefaultSearcherFactory;
use rucene::search::searcher_manager::SearcherManager;
use rucene::search::term_query::TermQuery;
use rucene::search::IndexSearcher;
use rucene::store::{ByteBuffersDirectory, Directory};
use rucene_test_framework::analysis::MockAnalyzer;

use crate::core_support::{assert_equals, class, random, test};

/// `TestTryDelete`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestTryDelete",
        vec![
            test("testTryDeleteDocument", test_try_delete_document),
            test(
                "testTryDeleteDocumentCloseAndReopen",
                test_try_delete_document_close_and_reopen,
            ),
            test("testDeleteDocuments", test_delete_documents),
        ],
    )
}

/// `private static IndexWriter getWriter(Directory directory)`.
fn get_writer(directory: &Arc<dyn Directory>) -> Result<Arc<IndexWriter>> {
    let policy = LogMergePolicy::by_byte_size();
    let conf = IndexWriterConfig::with_analyzer(Some(Arc::new(MockAnalyzer::new(&*random()))));
    conf.set_merge_policy(Some(Arc::new(policy)))?;
    conf.set_open_mode(OpenMode::CREATE_OR_APPEND)?;

    IndexWriter::new(Arc::clone(directory), conf)
}

/// `private static Directory createIndex()`.
fn create_index() -> Result<Arc<dyn Directory>> {
    let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());

    let writer = get_writer(&directory)?;

    for i in 0..10 {
        let mut doc = Document::new();
        doc.add(Box::new(StringField::new(
            "foo",
            i.to_string(),
            Store::YES,
        )?));
        writer.add_document(&doc)?;
    }

    writer.commit()?;
    writer.close()?;

    Ok(directory)
}

/// `new SearcherManager(writer, new SearcherFactory())`.
fn new_manager(writer: &IndexWriter) -> Result<SearcherManager> {
    SearcherManager::from_writer(
        writer,
        true,
        false,
        Some(Arc::new(DefaultSearcherFactory)),
        None,
    )
}

/// `new TermQuery(new Term("foo", "0"))`.
fn foo0() -> Arc<TermQuery> {
    Arc::new(TermQuery::new(Term::from_text("foo", "0")))
}

/// Port of `TestTryDelete.testTryDeleteDocument`.
fn test_try_delete_document() -> Result<()> {
    let directory = create_index()?;

    let writer = get_writer(&directory)?;

    let mgr = new_manager(&writer)?;

    let mut searcher = mgr.acquire()?;

    let mut top_docs = searcher.searcher().search(foo0(), 100)?;
    assert_equals(&1, &top_docs.total_hits.value());

    let result = if random().next_boolean() {
        let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
        let result = writer.try_delete_document(&r, 0)?;
        r.close()?;
        result
    } else {
        writer.try_delete_document(searcher.searcher().get_index_reader(), 0)?
    };

    // The tryDeleteDocument should have succeeded:
    Assert::assert_true(result != -1);

    Assert::assert_true(writer.has_deletions()?);

    if random().next_boolean() {
        writer.commit()?;
    }

    Assert::assert_true(writer.has_deletions()?);

    mgr.maybe_refresh()?;

    searcher = mgr.acquire()?;

    top_docs = searcher.searcher().search(foo0(), 100)?;

    assert_equals(&0, &top_docs.total_hits.value());
    Ok(())
}

/// Port of `TestTryDelete.testTryDeleteDocumentCloseAndReopen`.
fn test_try_delete_document_close_and_reopen() -> Result<()> {
    let directory = create_index()?;

    let writer = get_writer(&directory)?;

    let mgr = new_manager(&writer)?;

    let mut searcher = mgr.acquire()?;

    let mut top_docs = searcher.searcher().search(foo0(), 100)?;
    assert_equals(&1, &top_docs.total_hits.value());

    let r: Arc<dyn IndexReader> = directory_reader::open_with_writer(&*writer)?;
    let result = writer.try_delete_document(&r, 0)?;

    Assert::assert_true(result != -1);

    writer.commit()?;

    Assert::assert_true(writer.has_deletions()?);

    mgr.maybe_refresh()?;

    searcher = mgr.acquire()?;

    top_docs = searcher.searcher().search(foo0(), 100)?;

    assert_equals(&0, &top_docs.total_hits.value());

    writer.close()?;

    let searcher = IndexSearcher::new(directory_reader::open(Arc::clone(&directory))?)?;

    top_docs = searcher.search(foo0(), 100)?;

    assert_equals(&0, &top_docs.total_hits.value());
    Ok(())
}

/// Port of `TestTryDelete.testDeleteDocuments`.
fn test_delete_documents() -> Result<()> {
    let directory = create_index()?;

    let writer = get_writer(&directory)?;

    let mgr = new_manager(&writer)?;

    let mut searcher = mgr.acquire()?;

    let mut top_docs = searcher.searcher().search(foo0(), 100)?;
    assert_equals(&1, &top_docs.total_hits.value());

    let result = writer.delete_documents_by_queries(vec![foo0()])?;

    Assert::assert_true(result != -1);

    // writer.commit();

    Assert::assert_true(writer.has_deletions()?);

    mgr.maybe_refresh()?;

    searcher = mgr.acquire()?;

    top_docs = searcher.searcher().search(foo0(), 100)?;

    assert_equals(&0, &top_docs.total_hits.value());
    Ok(())
}
