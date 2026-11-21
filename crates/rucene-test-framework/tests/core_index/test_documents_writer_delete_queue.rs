//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocumentsWriterDeleteQueue.java`.
//!
//! Unit test for `DocumentsWriterDeleteQueue`.
//!
//! Java builds each queue with `new DocumentsWriterDeleteQueue(null)`, a
//! `null` info stream the queue never logs to; Rucene's constructor takes an
//! `InfoStream`, so the queues get `InfoStream.NO_OUTPUT`.

use std::collections::HashSet;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::doc_values_update::DocValuesUpdate;
use rucene::index::documents_writer::{
    BufferedUpdates, DeleteNode, DeleteSlice, DocumentsWriterDeleteQueue, FrozenBufferedUpdates,
};
use rucene::index::terms::Term;
use rucene::search::{MatchNoDocsQuery, Query, TermQuery};
use rucene::util::NoOutputInfoStream;
use rucene_test_framework::util::java_exceptions::expect_throws;
use rucene_test_framework::util::lucene_test_case::RANDOM_MULTIPLIER;

use crate::core_support::{assert_equals, assert_equals_msg, assert_null, class, random, test};
use crate::support::CountDownLatch;

/// `TestDocumentsWriterDeleteQueue`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocumentsWriterDeleteQueue",
        vec![
            test("testUpdateDeleteSlices", test_update_delete_slices),
            test("testClear", test_clear),
            test("testAnyChanges", test_any_changes),
            test(
                "testPartiallyAppliedGlobalSlice",
                test_partially_applied_global_slice,
            ),
            test("testStressDeleteQueue", test_stress_delete_queue),
            test("testClose", test_close),
        ],
    )
}

const ALREADY_CLOSED: &str = "org.apache.lucene.store.AlreadyClosedException";

/// `new DocumentsWriterDeleteQueue(null)`.
fn new_queue() -> DocumentsWriterDeleteQueue {
    DocumentsWriterDeleteQueue::new(Arc::new(NoOutputInfoStream))
}

/// `deletes.deleteTerms.keySet()`.
fn delete_term_set(deletes: &BufferedUpdates) -> HashSet<Term> {
    deletes.delete_terms().into_iter().map(|d| d.term).collect()
}

/// The terms of `frozen.deleteTerms`, read with its `TermIterator`.
fn frozen_term_set(frozen: &FrozenBufferedUpdates) -> Result<HashSet<Term>> {
    let mut frozen_set = HashSet::new();
    let mut iter = frozen.delete_terms_iterator();
    while let Some(bytes) = iter.next()? {
        let field = iter
            .field()
            .unwrap_or_else(|| crate::support::npe("iter.field()"))
            .to_string();
        frozen_set.insert(Term::new(
            field,
            rucene::util::BytesRef::deep_copy_of(&bytes),
        ));
    }
    Ok(frozen_set)
}

/// `queue.freezeGlobalBuffer(null)`.
fn freeze(queue: &DocumentsWriterDeleteQueue) -> Result<FrozenBufferedUpdates> {
    Ok(queue
        .freeze_global_buffer(None)?
        .unwrap_or_else(|| crate::support::npe("freezeGlobalBuffer(null)")))
}

/// Port of `TestDocumentsWriterDeleteQueue.testUpdateDeleteSlices`.
fn test_update_delete_slices() -> Result<()> {
    let queue = new_queue();
    let size = 200 + random().next_int_bound(500) * *RANDOM_MULTIPLIER;
    let mut ids = vec![0i32; size as usize];
    for id in ids.iter_mut() {
        *id = random().next_int();
    }
    let mut slice1 = queue.new_slice();
    let mut slice2 = queue.new_slice();
    let mut bd1 = BufferedUpdates::new("bd1");
    let mut bd2 = BufferedUpdates::new("bd2");
    let mut last1 = 0;
    let mut last2 = 0;
    let mut unique_values = HashSet::new();
    for j in 0..ids.len() {
        let i = ids[j];
        // create an array here since we compare identity below against tailItem
        let term = vec![Term::from_text("id", &i.to_string())];
        unique_values.insert(term[0].clone());
        let term_identity = term.as_ptr();
        queue.add_delete_terms(term)?;
        if random().next_int_bound(20) == 0 || j == ids.len() - 1 {
            queue.update_slice(&mut slice1)?;
            Assert::assert_true(slice1.is_tail_item_terms(term_identity));
            slice1.apply(&mut bd1, j as i32)?;
            assert_all_between(last1, j, &bd1, &ids);
            last1 = j + 1;
        }
        if random().next_int_bound(10) == 5 || j == ids.len() - 1 {
            queue.update_slice(&mut slice2)?;
            Assert::assert_true(slice2.is_tail_item_terms(term_identity));
            slice2.apply(&mut bd2, j as i32)?;
            assert_all_between(last2, j, &bd2, &ids);
            last2 = j + 1;
        }
        assert_equals(&unique_values.len(), &queue.num_global_term_deletes());
    }
    assert_equals(&unique_values, &delete_term_set(&bd1));
    assert_equals(&unique_values, &delete_term_set(&bd2));
    let frozen_set = frozen_term_set(&freeze(&queue)?)?;
    assert_equals(&unique_values, &frozen_set);
    assert_equals_msg(
        Some("num deletes must be 0 after freeze"),
        &0,
        &queue.num_global_term_deletes(),
    );
    Ok(())
}

/// `private void assertAllBetween(int start, int end, BufferedUpdates
/// deletes, Integer[] ids)`.
fn assert_all_between(start: usize, end: usize, deletes: &BufferedUpdates, ids: &[i32]) {
    for i in start..=end {
        assert_equals(
            &Some(end as i32),
            &deletes.delete_term_doc_id_upto(&Term::from_text("id", &ids[i].to_string())),
        );
    }
}

/// Port of `TestDocumentsWriterDeleteQueue.testClear`.
fn test_clear() -> Result<()> {
    let queue = new_queue();
    Assert::assert_false(queue.any_changes());
    queue.clear();
    Assert::assert_false(queue.any_changes());
    let size = 200 + random().next_int_bound(500) * *RANDOM_MULTIPLIER;
    for i in 0..size {
        let term = Term::from_text("id", &i.to_string());
        if random().next_int_bound(10) == 0 {
            queue.add_delete_queries(vec![Arc::new(TermQuery::new(term)) as Arc<dyn Query>])?;
        } else {
            queue.add_delete_terms(vec![term])?;
        }
        Assert::assert_true(queue.any_changes());
        if random().next_int_bound(10) == 0 {
            queue.clear();
            queue.try_apply_global_slice()?;
            Assert::assert_false(queue.any_changes());
        }
    }
    Ok(())
}

/// Port of `TestDocumentsWriterDeleteQueue.testAnyChanges`.
fn test_any_changes() -> Result<()> {
    let queue = new_queue();
    let size = 200 + random().next_int_bound(500) * *RANDOM_MULTIPLIER;
    let mut terms_since_freeze = 0;
    let mut queries_since_freeze = 0;
    for i in 0..size {
        let term = Term::from_text("id", &i.to_string());
        if random().next_int_bound(10) == 0 {
            queue.add_delete_queries(vec![Arc::new(TermQuery::new(term)) as Arc<dyn Query>])?;
            queries_since_freeze += 1;
        } else {
            queue.add_delete_terms(vec![term])?;
            terms_since_freeze += 1;
        }
        Assert::assert_true(queue.any_changes());
        if random().next_int_bound(5) == 0 {
            let freeze_global_buffer = freeze(&queue)?;
            assert_equals(
                &terms_since_freeze,
                &freeze_global_buffer.delete_terms_size(),
            );
            assert_equals(
                &(queries_since_freeze as usize),
                &freeze_global_buffer.updates().queries_size(),
            );
            queries_since_freeze = 0;
            terms_since_freeze = 0;
            Assert::assert_false(queue.any_changes());
        }
    }
    Ok(())
}

/// Port of `TestDocumentsWriterDeleteQueue.testPartiallyAppliedGlobalSlice`.
fn test_partially_applied_global_slice() -> Result<()> {
    let queue = Arc::new(new_queue());
    let lock = queue.global_buffer_lock();
    let t = {
        let queue = Arc::clone(&queue);
        Thread::unnamed(move || {
            if let Err(e) = queue.add_delete_terms(vec![Term::from_text("foo", "bar")]) {
                rucene_test_framework::util::java_exceptions::rethrow_java(e)
            }
        })
    };
    t.start();
    if let Err(payload) = t.join() {
        std::panic::resume_unwind(payload);
    }
    drop(lock);
    Assert::assert_true_msg(
        "changes in del queue but not in slice yet",
        queue.any_changes(),
    );
    queue.try_apply_global_slice()?;
    Assert::assert_true_msg("changes in global buffer", queue.any_changes());
    let freeze_global_buffer = freeze(&queue)?;
    Assert::assert_true(!freeze_global_buffer.is_empty());
    assert_equals(&1, &freeze_global_buffer.delete_terms_size());
    Assert::assert_false_msg("all changes applied", queue.any_changes());
    Ok(())
}

/// Port of `TestDocumentsWriterDeleteQueue.testStressDeleteQueue`.
fn test_stress_delete_queue() -> Result<()> {
    let queue = Arc::new(new_queue());
    let mut unique_values = HashSet::new();
    let size = 10000 + random().next_int_bound(500) * *RANDOM_MULTIPLIER;
    let mut ids = vec![0i32; size as usize];
    for id in ids.iter_mut() {
        *id = random().next_int();
        unique_values.insert(Term::from_text("id", &id.to_string()));
    }
    let ids = Arc::new(ids);
    let latch = Arc::new(CountDownLatch::new(1));
    let index = Arc::new(AtomicI32::new(0));
    let num_threads = 2 + random().next_int_bound(5);
    let mut threads = Vec::with_capacity(num_threads as usize);
    for _ in 0..num_threads {
        let update_thread = UpdateThread::new(&queue, &index, &ids, &latch);
        update_thread.thread.start();
        threads.push(update_thread);
    }
    latch.count_down();
    for update_thread in &threads {
        if let Err(payload) = update_thread.thread.join() {
            std::panic::resume_unwind(payload);
        }
    }

    for update_thread in &threads {
        let mut state = update_thread
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let (slice, deletes) = &mut *state;
        queue.update_slice(slice)?;
        slice.apply(deletes, BufferedUpdates::MAX_INT)?;
        assert_equals(&unique_values, &delete_term_set(deletes));
    }
    queue.try_apply_global_slice()?;

    let frozen_set = frozen_term_set(&freeze(&queue)?)?;

    assert_equals_msg(
        Some("num deletes must be 0 after freeze"),
        &0,
        &queue.num_global_term_deletes(),
    );
    assert_equals(&unique_values.len(), &frozen_set.len());
    assert_equals(&unique_values, &frozen_set);
    Ok(())
}

/// Port of `TestDocumentsWriterDeleteQueue.testClose`.
fn test_close() -> Result<()> {
    {
        let queue = new_queue();
        Assert::assert_true(queue.is_open());
        queue.close()?;
        if random().next_boolean() {
            queue.close()?; // double close
        }
        expect_throws(ALREADY_CLOSED, || {
            queue.add_delete_terms(vec![Term::from_text("foo", "bar")])
        });
        expect_throws(ALREADY_CLOSED, || queue.freeze_global_buffer(None));
        expect_throws(ALREADY_CLOSED, || {
            queue.add_delete_queries(vec![
                Arc::new(MatchNoDocsQuery::instance()) as Arc<dyn Query>
            ])
        });
        expect_throws(ALREADY_CLOSED, || {
            queue.add_doc_values_updates(vec![DocValuesUpdate::numeric(
                Term::from_text("foo", "bar"),
                "foo",
                1,
            )])
        });
        // `queue.add(null)`: the queue is closed before the node is read;
        // Rust takes a node, so any node stands for Java's `null`.
        expect_throws(ALREADY_CLOSED, || {
            queue.add(DeleteNode::new_term(Term::from_text("foo", "bar")))
        });
        assert_null(&queue.maybe_freeze_global_buffer()); // this is fine
        Assert::assert_false(queue.is_open());
    }
    {
        let queue = new_queue();
        queue.add_delete_terms(vec![Term::from_text("foo", "bar")])?;
        expect_throws("java.lang.IllegalStateException", || queue.close());
        Assert::assert_true(queue.is_open());
        queue.try_apply_global_slice()?;
        queue.freeze_global_buffer(None)?;
        queue.close()?;
        Assert::assert_false(queue.is_open());
    }
    Ok(())
}

/// `private static class UpdateThread extends Thread`: the thread, and its
/// `slice` and `deletes`, which the test reads after `join()`.
struct UpdateThread {
    thread: Thread,
    state: Arc<Mutex<(DeleteSlice, BufferedUpdates)>>,
}

impl UpdateThread {
    fn new(
        queue: &Arc<DocumentsWriterDeleteQueue>,
        index: &Arc<AtomicI32>,
        ids: &Arc<Vec<i32>>,
        latch: &Arc<CountDownLatch>,
    ) -> Self {
        let state = Arc::new(Mutex::new((
            queue.new_slice(),
            BufferedUpdates::new("deletes"),
        )));
        let thread = {
            let queue = Arc::clone(queue);
            let index = Arc::clone(index);
            let ids = Arc::clone(ids);
            let latch = Arc::clone(latch);
            let state = Arc::clone(&state);
            Thread::unnamed(move || {
                latch.await_();
                let run = || -> Result<()> {
                    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
                    let (slice, deletes) = &mut *state;
                    loop {
                        let i = index.fetch_add(1, Ordering::SeqCst);
                        if i as usize >= ids.len() {
                            break;
                        }
                        let term = Term::from_text("id", &ids[i as usize].to_string());
                        let term_node = DeleteNode::new_term(term);
                        queue.add_to_slice(Arc::clone(&term_node), slice)?;
                        Assert::assert_true(slice.is_tail(&term_node));
                        slice.apply(deletes, BufferedUpdates::MAX_INT)?;
                    }
                    Ok(())
                };
                if let Err(e) = run() {
                    rucene_test_framework::util::java_exceptions::rethrow_java(e)
                }
            })
        };
        UpdateThread { thread, state }
    }
}
