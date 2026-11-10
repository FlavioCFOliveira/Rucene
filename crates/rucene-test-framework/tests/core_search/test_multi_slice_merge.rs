//! Port of `lucene/core/src/test/org/apache/lucene/search/TestMultiSliceMerge.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::{SortedDocValuesField, Store};
use rucene::error::Result;
use rucene::index::directory_reader::DirectoryReader;
use rucene::index::IndexReader;
use rucene::search::index_searcher::IndexSearcher;
use rucene::search::{
    Command, Executor, MatchAllDocsQuery, Query, RejectedExecutionException, TopDocs,
};
use rucene::util::BytesRef;
use rucene_test_framework::index::random_index_writer::RandomIndexWriter;
use rucene_test_framework::search::check_hits::CheckHits;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_search::support::{fixture_class, ftest, set_up, tear_down};
use crate::core_support::{assert_equals, random};

/// The instance of `TestMultiSliceMerge`.
#[derive(Default)]
pub struct MultiSliceMerge {
    dir1: Option<TestDirectory>,
    dir2: Option<TestDirectory>,
    reader1: Option<Arc<dyn DirectoryReader>>,
    reader2: Option<Arc<dyn DirectoryReader>>,
}

/// `TestMultiSliceMerge`.
pub fn class_def() -> Arc<Class> {
    fixture_class::<MultiSliceMerge>(
        "org.apache.lucene.search.TestMultiSliceMerge",
        vec![
            set_up(multi_slice_set_up),
            tear_down(multi_slice_tear_down),
            ftest(
                "testMultipleSlicesOfSameIndexSearcher",
                test_multiple_slices_of_same_index_searcher,
            ),
            ftest(
                "testMultipleSlicesOfMultipleIndexSearchers",
                test_multiple_slices_of_multiple_index_searchers,
            ),
        ],
    )
}

/// One `RandomIndexWriter` of `setUp`: 100 documents, `between` after each.
fn index(
    dir: &TestDirectory,
    random: &dyn randomizedtesting::jdk::random::Random,
    between: impl Fn(&RandomIndexWriter) -> Result<()>,
) -> Result<Arc<dyn DirectoryReader>> {
    let mut iwc = LuceneTestCase::new_index_writer_config();
    iwc.set_merge_policy(Arc::new(LuceneTestCase::new_log_merge_policy()))?;
    let iw = RandomIndexWriter::new_with_config(&*self::random(), dir.as_directory(), iwc)?;
    for i in 0..100 {
        let mut doc = rucene::document::Document::new();
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field",
            &i.to_string(),
            Store::NO,
        )));
        doc.add(Box::new(LuceneTestCase::new_string_field(
            "field2",
            &(i % 2 == 0).to_string(),
            Store::NO,
        )));
        doc.add(Box::new(SortedDocValuesField::new(
            "field2",
            BytesRef::from_char_sequence(&(i % 2 == 0).to_string()),
        )));
        iw.add_document(&doc)?;
        if random.next_boolean() {
            between(&iw)?;
        }
    }
    let reader = iw.get_reader()?;
    iw.close()?;
    Ok(reader)
}

fn multi_slice_set_up(f: &mut MultiSliceMerge) -> Result<()> {
    let dir1 = LuceneTestCase::new_directory();
    let dir2 = LuceneTestCase::new_directory();
    let random = random();
    f.reader1 = Some(index(&dir1, &*random, |iw| iw.get_reader()?.close())?);
    f.reader2 = Some(index(&dir2, &*random, |iw| iw.commit().map(|_| ()))?);
    f.dir1 = Some(dir1);
    f.dir2 = Some(dir2);
    Ok(())
}

fn multi_slice_tear_down(f: &mut MultiSliceMerge) -> Result<()> {
    for reader in [f.reader1.take(), f.reader2.take()].into_iter().flatten() {
        reader.close()?;
    }
    for mut dir in [f.dir1.take(), f.dir2.take()].into_iter().flatten() {
        dir.close()?;
    }
    Ok(())
}

/// The lambda `runnable -> runnable.run()`.
struct DirectExecutor;

impl Executor for DirectExecutor {
    fn execute(&self, command: Command) -> std::result::Result<(), RejectedExecutionException> {
        command();
        Ok(())
    }
}

fn searchers(f: &MultiSliceMerge) -> Result<[IndexSearcher; 2]> {
    let reader = |r: &Option<Arc<dyn DirectoryReader>>| {
        Arc::clone(r.as_ref().expect("setUp ran")) as Arc<dyn IndexReader>
    };
    Ok([
        IndexSearcher::with_executor(reader(&f.reader1), Some(Arc::new(DirectExecutor)))?,
        IndexSearcher::with_executor(reader(&f.reader2), Some(Arc::new(DirectExecutor)))?,
    ])
}

fn test_multiple_slices_of_same_index_searcher(f: &mut MultiSliceMerge) -> Result<()> {
    let searchers = searchers(f)?;
    let query: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let top_docs1 = searchers[0].search(Arc::clone(&query), i32::MAX)?;
    let top_docs2 = searchers[1].search(Arc::clone(&query), i32::MAX)?;
    CheckHits::check_equal(&*query, &top_docs1.score_docs, &top_docs2.score_docs);
    Ok(())
}

fn test_multiple_slices_of_multiple_index_searchers(f: &mut MultiSliceMerge) -> Result<()> {
    let searchers = searchers(f)?;
    let query: Arc<dyn Query> = Arc::new(MatchAllDocsQuery::instance());
    let mut top_docs1 = searchers[0].search(Arc::clone(&query), i32::MAX)?;
    let mut top_docs2 = searchers[1].search(Arc::clone(&query), i32::MAX)?;

    assert_equals(&top_docs1.score_docs.len(), &top_docs2.score_docs.len());

    for i in 0..top_docs1.score_docs.len() {
        top_docs1.score_docs[i].shard_index = 0;
        top_docs2.score_docs[i].shard_index = 1;
    }

    let n = top_docs1.score_docs.len();
    let shard_hits: [TopDocs; 2] = [top_docs1, top_docs2];

    let merged_hits1 = TopDocs::merge_from(0, n, &shard_hits)?;
    let merged_hits2 = TopDocs::merge_from(0, n, &shard_hits)?;
    CheckHits::check_equal(&*query, &merged_hits1.score_docs, &merged_hits2.score_docs);
    Ok(())
}
