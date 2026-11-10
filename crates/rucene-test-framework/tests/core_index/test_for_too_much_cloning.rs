//! Port of `lucene/core/src/test/org/apache/lucene/index/TestForTooMuchCloning.java`.

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store, TextField};
use rucene::error::Result;
use rucene::index::merge_policy::{OneMerge, OneMergeWrappingMergePolicy, TieredMergePolicy};
use rucene::index::merge_scheduler::SerialMergeScheduler;
use rucene::index::IndexReader;
use rucene::search::term_range_query::TermRangeQuery;
use rucene::util::BytesRef;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::index::RandomIndexWriter;
use rucene_test_framework::util::lucene_test_case::TestDirectory;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test};

/// `TestForTooMuchCloning`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestForTooMuchCloning",
        vec![test("test", test_)],
    )
}

/// Make sure we don't clone IndexInputs too frequently
/// during merging and searching:
/// Port of `TestForTooMuchCloning.test`.
fn test_() -> Result<()> {
    let mut dir = TestDirectory::Mock(LuceneTestCase::new_mock_directory());
    let mock = dir.as_mock().unwrap_or_else(|| crate::support::npe("dir"));
    mock.set_verbose_clone(false); // set true to view clone stacks.
    let tmp = TieredMergePolicy::new();
    tmp.set_max_merge_at_once(2)?;
    let segments_merged = Arc::new(AtomicI32::new(0));
    let mut iwc =
        LuceneTestCase::new_index_writer_config_analyzer(Arc::new(MockAnalyzer::new(&*random())));
    // to reduce flakiness on merge clone count
    iwc.set_merge_scheduler(Arc::new(SerialMergeScheduler::new()))?;
    iwc.set_max_buffered_docs(2)?;
    {
        let segments_merged = Arc::clone(&segments_merged);
        iwc.set_merge_policy(Arc::new(OneMergeWrappingMergePolicy::new(
            Arc::new(tmp),
            Arc::new(move |one_merge: OneMerge| {
                segments_merged.fetch_add(one_merge.segments.len() as i32, Ordering::SeqCst);
                one_merge
            }),
        )))?;
    }
    let w = RandomIndexWriter::new_with_config(&*random(), dir.as_directory(), iwc)?;
    let num_docs = 20;
    for _ in 0..num_docs {
        let mut sb = String::new();
        for _ in 0..100 {
            sb.push_str(&TestUtil::random_realistic_unicode_string(&*random()));
            sb.push(' ');
        }
        let mut doc = Document::new();
        doc.add(Box::new(TextField::new("field", sb, Store::NO)?));
        w.add_document(&doc)?;
    }
    let dr = w.get_reader()?;
    let r: Arc<dyn IndexReader> = Arc::clone(&dr) as Arc<dyn IndexReader>;
    w.close()?;
    // System.out.println("merge clone count=" + cloneCount);
    let mock = dir.as_mock().unwrap_or_else(|| crate::support::npe("dir"));
    Assert::assert_true_msg(
        &format!(
            "too many calls to IndexInput.clone during merging: {}",
            mock.get_input_clone_count()
        ),
        mock.get_input_clone_count()
            < (Arc::clone(&r).leaves()?.len() as i32 + segments_merged.load(Ordering::SeqCst)) * 50,
    );

    let s = LuceneTestCase::new_searcher(dr);
    // important: set this after newSearcher, it might have run checkindex
    let clone_count = mock.get_input_clone_count();
    // dir.setVerboseClone(true);

    // MTQ that matches all terms so the AUTO_REWRITE should
    // cutover to filter rewrite and reuse a single DocsEnum
    // across all terms;
    let hits = s.search(
        Arc::new(TermRangeQuery::new(
            "field",
            Some(BytesRef::new(Vec::new())),
            Some(BytesRef::from_char_sequence("\u{FFFF}")),
            true,
            true,
        )?),
        10,
    )?;
    Assert::assert_true(hits.total_hits.value() > 0);
    let query_clone_count = mock.get_input_clone_count() - clone_count;
    // System.out.println("query clone count=" + queryCloneCount);
    // It is rather difficult to reliably predict how many query clone calls will be performed. One
    // important factor is the number of segment partitions being searched, but it depends as well
    // on the terms being indexed, and the distribution of the matches across the documents, which
    // affects how the query gets rewritten and the subsequent number of clone calls it will
    // perform.
    Assert::assert_true_msg(
        &format!("too many calls to IndexInput.clone during TermRangeQuery: {query_clone_count}"),
        query_clone_count <= (s.get_leaf_contexts().len().max(s.get_slices()?.len()) as i32) * 7,
    );
    r.close()?;
    drop(s);
    dir.close()
}
