//! Port of `lucene/core/src/test/org/apache/lucene/index/TestDocumentsWriterPerThreadPool.java`.
//!
//! Java's factory builds `new DocumentsWriterPerThread(Version.LATEST.major,
//! "", directory, directory, newIndexWriterConfig(), new
//! DocumentsWriterDeleteQueue(null), null, new AtomicLong(), false)`, with a
//! `null` field-infos builder the DWPT never uses before it indexes and the
//! indexing chain the DWPT builds itself. Rucene's constructor takes the
//! builder and the chain, so the factory hands it an empty builder and the
//! default chain over the same configuration; the delete queue gets
//! `InfoStream.NO_OUTPUT` for Java's `null`.

use std::sync::atomic::AtomicI64;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::{State, Thread};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::documents_writer::{
    DocumentsWriterDeleteQueue, DocumentsWriterPerThread, DocumentsWriterPerThreadPool,
};
use rucene::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use rucene::index::DefaultIndexingChain;
use rucene::store::Directory;
use rucene::util::{NoOutputInfoStream, Version};
use rucene_test_framework::util::java_exceptions::is_instance;
use rucene_test_framework::util::LuceneTestCase;

use crate::core_support::{assert_equals, class, test};
use crate::support::CountDownLatch;

/// `TestDocumentsWriterPerThreadPool`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestDocumentsWriterPerThreadPool",
        vec![
            test("testLockReleaseAndClose", test_lock_release_and_close),
            test(
                "testCloseWhileNewWritersLocked",
                test_close_while_new_writers_locked,
            ),
        ],
    )
}

/// `new DocumentsWriterPerThreadPool(() -> new DocumentsWriterPerThread(…))`.
fn new_pool(directory: Arc<dyn Directory>) -> DocumentsWriterPerThreadPool {
    DocumentsWriterPerThreadPool::new(move || {
        let config = LuceneTestCase::new_index_writer_config().into_live();
        DocumentsWriterPerThread::new(
            i32::from(Version::LATEST.major),
            String::new(),
            Arc::clone(&directory),
            Arc::clone(&directory),
            Arc::clone(&config),
            Arc::new(DocumentsWriterDeleteQueue::new(Arc::new(
                NoOutputInfoStream,
            ))),
            FieldInfosBuilder::new(Arc::new(FieldNumbers::new(None, None)?)),
            Arc::new(AtomicI64::new(0)),
            false,
            Box::new(DefaultIndexingChain::new(config)),
        )
    })
}

/// Port of `TestDocumentsWriterPerThreadPool.testLockReleaseAndClose`.
fn test_lock_release_and_close() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    {
        let pool = new_pool(directory.as_directory());

        let first = pool.get_and_lock()?;
        let first_dwpt = Arc::clone(first.dwpt());
        assert_equals(&1, &pool.size());
        let second = pool.get_and_lock()?;
        assert_equals(&2, &pool.size());
        pool.mark_as_free_and_unlock(first);
        assert_equals(&2, &pool.size());
        let third = pool.get_and_lock()?;
        // `assertSame(first, third)`
        Assert::assert_true(Arc::ptr_eq(&first_dwpt, third.dwpt()));
        assert_equals(&2, &pool.size());
        pool.checkout(&third);
        assert_equals(&1, &pool.size());

        pool.close();
        assert_equals(&1, &pool.size());
        pool.mark_as_free_and_unlock(second);
        assert_equals(&1, &pool.size());
        for last_per_thead in pool.filter_and_lock(|_| true) {
            pool.checkout(&last_per_thead);
            last_per_thead.unlock();
        }
        assert_equals(&0, &pool.size());
        drop(third);
    }
    directory.close()
}

/// Port of `TestDocumentsWriterPerThreadPool.testCloseWhileNewWritersLocked`.
fn test_close_while_new_writers_locked() -> Result<()> {
    let directory = LuceneTestCase::new_directory();
    {
        let pool = Arc::new(new_pool(directory.as_directory()));

        let first = pool.get_and_lock()?;
        pool.lock_new_writers();
        let latch = Arc::new(CountDownLatch::new(1));
        let t = {
            let pool = Arc::clone(&pool);
            let latch = Arc::clone(&latch);
            Thread::unnamed(move || {
                latch.count_down();
                match pool.get_and_lock() {
                    Ok(_) => Assert::fail_without_message(),
                    Err(e) if is_instance(&e, "org.apache.lucene.store.AlreadyClosedException") => {
                        // fine
                    }
                    Err(e) => rucene_test_framework::util::java_exceptions::rethrow_java(e),
                }
            })
        };
        t.start();
        latch.await_();
        while t.state() != State::Waiting {
            std::thread::yield_now();
        }
        first.unlock();
        pool.close();
        pool.unlock_new_writers();
        for per_thread in pool.filter_and_lock(|_| true) {
            Assert::assert_true(pool.checkout(&per_thread));
            per_thread.unlock();
        }
        assert_equals(&0, &pool.size());
    }
    directory.close()
}
