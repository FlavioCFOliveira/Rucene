//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMultiIndexMergeScheduler.java`.
//!
//! * The anonymous `CombinedMergeScheduler` subclasses overriding
//!   `sync(Directory)` install that override with
//!   `CombinedMergeScheduler::set_sync_override`; `super.sync(directory)` is
//!   `super_sync_directory`.
//! * Java's sets of directories compare them by identity (`Directory`
//!   inherits `Object.equals`); so does [`DirectorySet`], over the one
//!   `Arc<dyn Directory>` handle each index is given.
//! * The `synchronized (lock)`/`wait`/`notifyAll` monitor is a `Mutex` with a
//!   `Condvar`; the state it guards (`nextDirectoryToWrite`,
//!   `documentCounts`, `writers`) lives inside the mutex.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use randomizedtesting::annotations::Timeout;
use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::document::{Document, Store};
use rucene::error::Result;
use rucene::index::index_writer::IndexWriter;
use rucene::index::index_writer_config::IndexWriterConfig;
use rucene::index::merge_scheduler::{
    CombinedMergeScheduler, MergeScheduler, MultiIndexMergeScheduler,
};
use rucene::store::Directory;
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{class, random, test};

/// `TestMultiIndexMergeScheduler`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMultiIndexMergeScheduler",
        vec![
            test("testCloseSingle", test_close_single),
            test("testCloseMultiple", test_close_multiple).annotate(Timeout {
                millis: 1000 * 60 * 2,
            }),
            test("testReferenceCounting", test_reference_counting),
        ],
    )
}

/// A set of directories compared by identity.
#[derive(Default)]
struct DirectorySet(Mutex<Vec<Arc<dyn Directory>>>);

impl DirectorySet {
    fn add(&self, directory: &Arc<dyn Directory>) {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(Arc::clone(directory));
    }

    /// `remove(directory)`: whether it was present.
    fn remove(&self, directory: &Arc<dyn Directory>) -> bool {
        let mut set = self.0.lock().unwrap_or_else(|p| p.into_inner());
        match set
            .iter()
            .position(|d| std::ptr::addr_eq(Arc::as_ptr(d), Arc::as_ptr(directory)))
        {
            Some(i) => {
                set.remove(i);
                true
            }
            None => false,
        }
    }

    fn is_empty(&self) -> bool {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).is_empty()
    }
}

/// A `CombinedMergeScheduler` whose `sync(directory)` calls
/// `super.sync(directory)`, then asserts `set.remove(directory)`.
fn removing_combined_scheduler(set: &Arc<DirectorySet>) -> Arc<CombinedMergeScheduler> {
    let combined = Arc::new(CombinedMergeScheduler::new());
    let set = Arc::clone(set);
    combined.set_sync_override(Some(Arc::new(move |this, directory| {
        this.super_sync_directory(directory);
        Assert::assert_true(set.remove(directory));
    })));
    combined
}

/// Port of `TestMultiIndexMergeScheduler.testCloseSingle`.
fn test_close_single() -> Result<()> {
    let dir = LuceneTestCase::new_directory();
    let directory = dir.as_directory();
    let directories_to_be_closed = Arc::new(DirectorySet::default());
    directories_to_be_closed.add(&directory);
    let scheduler = Arc::new(MultiIndexMergeScheduler::with_combined_merge_scheduler(
        Arc::clone(&directory),
        removing_combined_scheduler(&directories_to_be_closed),
    ));
    scheduler
        .get_combined_merge_scheduler()
        .set_default_max_merges_and_threads(false);

    let iwc = IndexWriterConfig::new();
    iwc.set_merge_scheduler(Some(Arc::clone(&scheduler) as Arc<dyn MergeScheduler>))?;
    iwc.set_max_buffered_docs(2)?;
    let writer = IndexWriter::new(Arc::clone(&directory), iwc)?;
    for _ in 0..5 {
        let mut doc = Document::new();
        let id_field = LuceneTestCase::new_string_field("id", "", Store::YES);
        doc.add(Box::new(id_field));
        writer.add_document(&doc)?;
    }
    writer.commit()?;
    writer.force_merge(1)?;
    writer.close()?;
    drop(writer);
    drop(scheduler);
    drop(directory);
    dir.close()?;
    Assert::assert_true(directories_to_be_closed.is_empty());
    Ok(())
}

/// The state Java's threads share under `synchronized (lock)`.
struct Shared {
    next_directory_to_write: VecDeque<i32>,
    document_counts: Vec<i32>,
    writers: Vec<Option<Arc<IndexWriter>>>,
}

/// Port of `TestMultiIndexMergeScheduler.testCloseMultiple`
/// (`@Timeout(millis = 1000 * 60 * 2)`).
fn test_close_multiple() -> Result<()> {
    const DIRECTORY_COUNT: usize = 10;
    const DOCUMENT_COUNT: usize = 50;

    let directories = Arc::new(DirectorySet::default());
    let mut test_dirs = Vec::with_capacity(DIRECTORY_COUNT);
    let mut directories_to_be_closed: Vec<Arc<dyn Directory>> = Vec::with_capacity(DIRECTORY_COUNT);
    for _ in 0..DIRECTORY_COUNT {
        let dir = LuceneTestCase::new_directory();
        let directory = dir.as_directory();
        directories.add(&directory);
        directories_to_be_closed.push(directory);
        test_dirs.push(dir);
    }

    let combined_scheduler = removing_combined_scheduler(&directories);

    let mut schedulers: Vec<Arc<MultiIndexMergeScheduler>> = Vec::new();
    let mut writers: Vec<Option<Arc<IndexWriter>>> = Vec::new();
    let mut document_counts = vec![0i32; DIRECTORY_COUNT];
    for (directory_index, directory) in directories_to_be_closed.iter().enumerate() {
        let scheduler = Arc::new(MultiIndexMergeScheduler::with_combined_merge_scheduler(
            Arc::clone(directory),
            Arc::clone(&combined_scheduler),
        ));
        schedulers.push(Arc::clone(&scheduler));
        let iwc = IndexWriterConfig::new();
        iwc.set_merge_scheduler(Some(Arc::clone(&scheduler) as Arc<dyn MergeScheduler>))?;
        iwc.set_max_buffered_docs(2)?;
        let writer = IndexWriter::new(Arc::clone(scheduler.get_directory()), iwc)?;
        writers.push(Some(writer));
        document_counts[directory_index] = 0;
    }
    Assert::assert_true(schedulers.len() == DIRECTORY_COUNT);
    Assert::assert_true(writers.len() == DIRECTORY_COUNT);

    schedulers[0]
        .get_combined_merge_scheduler()
        .set_default_max_merges_and_threads(false);

    let mut next_directory_to_write: VecDeque<i32> = VecDeque::new();
    for _ in 0..DOCUMENT_COUNT {
        let r = random().next_int_bound(DIRECTORY_COUNT as i32);
        next_directory_to_write.push_back(r);
        document_counts[r as usize] += 1;
    }
    for count in document_counts.iter_mut() {
        *count = (*count).max(1);
    }

    let stop = Arc::new(AtomicBool::new(false));
    let mut threads = Vec::new();
    let lock = Arc::new((
        Mutex::new(Shared {
            next_directory_to_write,
            document_counts,
            writers,
        }),
        Condvar::new(),
    ));
    for scheduler_index in 0..schedulers.len() {
        let thread_id = scheduler_index as i32;
        let (stop, lock) = (Arc::clone(&stop), Arc::clone(&lock));
        let thread = Thread::unnamed(move || {
            let outcome = (|| -> Result<()> {
                let writer = lock.0.lock().unwrap_or_else(|p| p.into_inner()).writers
                    [thread_id as usize]
                    .clone();
                while !stop.load(Ordering::SeqCst) {
                    let (mutex, cond) = &*lock;
                    let mut shared = mutex.lock().unwrap_or_else(|p| p.into_inner());
                    if shared
                        .next_directory_to_write
                        .front()
                        .is_some_and(|&next| next != thread_id)
                    {
                        while shared
                            .next_directory_to_write
                            .front()
                            .is_some_and(|&next| next != thread_id)
                        {
                            shared = cond.wait(shared).unwrap_or_else(|p| p.into_inner());
                        }
                        continue;
                    }
                    if shared.document_counts[thread_id as usize] == 0 {
                        break;
                    }
                    let writer = writer
                        .as_ref()
                        .unwrap_or_else(|| crate::support::npe("writer"));
                    let mut doc = Document::new();
                    let id_field = LuceneTestCase::new_string_field("id", "", Store::NO);
                    doc.add(Box::new(id_field));
                    writer.add_document(&doc)?;
                    shared.document_counts[thread_id as usize] -= 1;
                    if shared.document_counts[thread_id as usize] == 0 {
                        writer.commit()?;
                        writer.force_merge_with(1, true)?;
                        writer.commit()?;
                        writer.close()?;
                        shared.writers[thread_id as usize] = None;
                    }
                    shared.next_directory_to_write.pop_front();
                    cond.notify_all();
                }
                Ok(())
            })();
            if let Err(e) = outcome {
                Throwable::from_cause("java.lang.RuntimeException", java_throwable(&e)).throw();
            }
        });
        thread.set_name(format!("TestMultiIndexMergeScheduler thread-{thread_id}"));
        thread.start();
        threads.push(thread);
    }

    for thread in &threads {
        let _ = thread.join();
    }

    for writer in &lock.0.lock().unwrap_or_else(|p| p.into_inner()).writers {
        Assert::assert_true(writer.is_none());
    }
    Assert::assert_true(directories.is_empty());

    drop(schedulers);
    drop(combined_scheduler);
    drop(directories_to_be_closed);
    for dir in &mut test_dirs {
        dir.close()?;
    }
    Ok(())
}

/// Port of `TestMultiIndexMergeScheduler.testReferenceCounting`.
fn test_reference_counting() -> Result<()> {
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_none()); // 0

    let directory1 = LuceneTestCase::new_directory();
    let mims1 = MultiIndexMergeScheduler::new(directory1.as_directory());
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_some()); // 1

    let directory2 = LuceneTestCase::new_directory();
    let mims2 = MultiIndexMergeScheduler::new(directory2.as_directory());
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_some()); // 2

    mims1.close()?;
    drop(mims1);
    directory1.close()?;
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_some()); // 1

    mims2.close()?;
    drop(mims2);
    directory2.close()?;
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_none()); // 0

    let directory3 = LuceneTestCase::new_directory();
    let mims3 = MultiIndexMergeScheduler::new(directory3.as_directory());
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_some()); // 1

    mims3.close()?;
    drop(mims3);
    directory3.close()?;
    Assert::assert_true(CombinedMergeScheduler::peek_singleton().is_none()); // 0
    Ok(())
}
