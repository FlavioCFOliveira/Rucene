//! Port of `lucene/core/src/test/org/apache/lucene/index/TestForceMergeForever.java`.
//!
//! * `MyIndexWriter extends IndexWriter` is an `IndexWriter` with the
//!   [`IndexWriterOverrides`] of its `merge(OneMerge)` override installed.

use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, IndexWriterOverrides, MAX_TERM_LENGTH};
use rucene::index::merge_policy::{LogMergePolicy, OneMerge, TieredMergePolicy};
use rucene::index::merge_scheduler::ConcurrentMergeScheduler;
use rucene::index::terms::Term;
use rucene_test_framework::analysis::MockAnalyzer;
use rucene_test_framework::util::java_exceptions::rethrow_java;
use rucene_test_framework::util::{LineFileDocs, LuceneTestCase, TestUtil};

use crate::core_support::{class, random, test, verbose};

/// `TestForceMergeForever`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestForceMergeForever",
        vec![test("test", test_)],
    )
}

/// Just counts how many merges are done
///
/// (`private static class MyIndexWriter extends IndexWriter`: its
/// `merge(OneMerge)` override.)
struct MyIndexWriter {
    merge_count: Arc<AtomicI32>,
    first: AtomicBool,
}

impl IndexWriterOverrides for MyIndexWriter {
    fn merge(&self, writer: &IndexWriter, merge: OneMerge) -> Result<()> {
        if merge.max_num_segments() != -1
            && (self.first.load(Ordering::SeqCst) || merge.segments.len() == 1)
        {
            self.first.store(false, Ordering::SeqCst);
            if verbose() {
                println!("TEST: maxNumSegments merge");
            }
            self.merge_count.fetch_add(1, Ordering::SeqCst);
        }
        writer.super_merge(merge)
    }
}

/// Port of `TestForceMergeForever.test`.
fn test_() -> Result<()> {
    let mut d = LuceneTestCase::new_directory();
    let analyzer = MockAnalyzer::new(&*random());
    analyzer.set_max_token_length(TestUtil::next_int(&*random(), 1, MAX_TERM_LENGTH as i32));
    let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(analyzer)));
    // SMS can cause this test to run indefinitely long:
    iwc.set_merge_scheduler(Some(Arc::new(ConcurrentMergeScheduler::new())))?;

    let w = IndexWriter::new(d.as_directory(), iwc)?;
    let merge_count = Arc::new(AtomicI32::new(0));
    w.set_overrides(Arc::new(MyIndexWriter {
        merge_count: Arc::clone(&merge_count),
        first: AtomicBool::new(false),
    }));

    // Try to make an index that requires merging:
    w.get_config()
        .set_max_buffered_docs(TestUtil::next_int(&*random(), 2, 11))?;
    let num_start_docs = LuceneTestCase::at_least(20);
    let docs = Arc::new(LineFileDocs::with_default_file(&*random())?);
    for _ in 0..num_start_docs {
        w.add_document(&docs.next_doc()?)?;
    }
    let mp = w.get_config().merge_policy();
    let merge_at_once = 1 + w.clone_segment_infos()?.size() as i32;
    if let Some(tmp) = mp
        .as_any()
        .and_then(|a| a.downcast_ref::<TieredMergePolicy>())
    {
        tmp.set_max_merge_at_once(merge_at_once)?;
    } else if let Some(lmp) = mp.as_any().and_then(|a| a.downcast_ref::<LogMergePolicy>()) {
        lmp.set_merge_factor(merge_at_once)?;
    } else {
        // skip test
        w.close()?;
        return d.close();
    }

    let do_stop = Arc::new(AtomicBool::new(false));
    w.get_config().set_max_buffered_docs(2)?;
    let t = {
        let w = Arc::clone(&w);
        let do_stop = Arc::clone(&do_stop);
        let docs = Arc::clone(&docs);
        Thread::unnamed(move || {
            let result = (|| -> Result<()> {
                while !do_stop.load(Ordering::SeqCst) {
                    w.update_document(
                        Term::from_text(
                            "docid",
                            &random().next_int_bound(num_start_docs).to_string(),
                        ),
                        &docs.next_doc()?,
                    )?;
                    // Force deletes to apply
                    directory_reader::open_with_writer(&*w)?.close()?;
                }
                Ok(())
            })();
            if let Err(t) = result {
                rethrow_java(LuceneError::runtime_exception(t));
            }
        })
    };
    t.start();
    w.force_merge(1)?;
    do_stop.store(true, Ordering::SeqCst);
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
    Assert::assert_true_msg(
        &format!("merge count is {}", merge_count.load(Ordering::SeqCst)),
        merge_count.load(Ordering::SeqCst) <= 1,
    );
    w.close()?;
    d.close()?;
    docs.close();
    Ok(())
}
