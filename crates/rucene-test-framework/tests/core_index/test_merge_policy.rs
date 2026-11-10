//! Port of `lucene/core/src/test/org/apache/lucene/index/TestMergePolicy.java`.
//!
//! * The specification the test thread and the closing thread share is an
//!   `Arc<MergeSpecification>`.
//! * `new SegmentInfo(…, null /* codec */, …, null /* indexSort */)` is
//!   `SegmentInfo::new_without_codec` with the default `Sort`, the value an
//!   unsorted segment holds.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::merge_policy::{MergeSpecification, OneMerge};
use rucene::index::{SegmentCommitInfo, SegmentInfo};
use rucene::search::sort::Sort;
use rucene::store::Directory;
use rucene::util::string_helper::{StringHelper, ID_LENGTH};
use rucene::util::Version;
use rucene_test_framework::util::java_exceptions::{expect_throws, java_throwable};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};

const ISE: &str = "java.lang.IllegalStateException";

/// `TestMergePolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestMergePolicy",
        vec![
            test("testWaitForOneMerge", test_wait_for_one_merge),
            test("testTimeout", test_timeout),
            // `@SuppressForbidden(reason = "Thread sleep")`: a forbidden-APIs
            // marker, with no runtime effect.
            test(
                "testTimeoutLargeNumberOfMerges",
                test_timeout_large_number_of_merges,
            ),
            test("testFinishTwice", test_finish_twice),
            test("testTotalMaxDoc", test_total_max_doc),
        ],
    )
}

/// `try (Directory dir = newDirectory()) { body }`.
fn with_dir(body: impl FnOnce(Arc<dyn Directory>) -> Result<()>) -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let outcome = body(dir.as_directory());
    let closed = dir.close();
    outcome?;
    closed
}

/// `m.close(true, false, mr -> {})` in a thread, where an `IOException`
/// becomes `new AssertionError(e)`.
fn close_or_assertion_error(m: &OneMerge) {
    if let Err(e) = m.close(true, false, |_mr| Ok(())) {
        Throwable::from_cause("java.lang.AssertionError", java_throwable(&e)).throw();
    }
}

/// Port of `TestMergePolicy.testWaitForOneMerge`.
fn test_wait_for_one_merge() -> Result<()> {
    with_dir(|dir| {
        let ms = Arc::new(create_random_merge_specification(
            &dir,
            1 + random().next_int_bound(10),
        )?);
        for m in &ms.merges {
            Assert::assert_false(m.has_completed_successfully().is_some());
        }
        let ms2 = Arc::clone(&ms);
        let t = Thread::unnamed(move || {
            for m in &ms2.merges {
                close_or_assertion_error(m);
            }
        });
        t.start();
        // `ms.await(100, TimeUnit.HOURS)`
        Assert::assert_true(ms.r#await(100 * 60 * 60 * 1000));
        for m in &ms.merges {
            Assert::assert_true(m.has_completed_successfully().unwrap_or_else(|| {
                Throwable::new("java.util.NoSuchElementException", "No value present").throw()
            }));
        }
        let _ = t.join();
        Ok(())
    })
}

/// Port of `TestMergePolicy.testTimeout`.
fn test_timeout() -> Result<()> {
    with_dir(|dir| {
        let ms = Arc::new(create_random_merge_specification(&dir, 3)?);
        for m in &ms.merges {
            Assert::assert_false(m.has_completed_successfully().is_some());
        }
        let ms2 = Arc::clone(&ms);
        let t = Thread::unnamed(move || {
            close_or_assertion_error(&ms2.merges[0]);
        });
        t.start();
        Assert::assert_false(ms.r#await(10));
        Assert::assert_false(ms.merges[1].has_completed_successfully().is_some());
        let _ = t.join();
        Ok(())
    })
}

/// Port of `TestMergePolicy.testTimeoutLargeNumberOfMerges`.
fn test_timeout_large_number_of_merges() -> Result<()> {
    with_dir(|dir| {
        let ms = Arc::new(create_random_merge_specification(&dir, 10000)?);
        for m in &ms.merges {
            Assert::assert_false(m.has_completed_successfully().is_some());
        }
        let i = Arc::new(AtomicI32::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let (ms2, i2, stop2) = (Arc::clone(&ms), Arc::clone(&i), Arc::clone(&stop));
        let t = Thread::unnamed(move || {
            while !stop2.load(Ordering::SeqCst) {
                let index = i2.fetch_add(1, Ordering::SeqCst) as usize;
                // `ms.merges.get(i)`: past the end, an IndexOutOfBoundsException.
                let m = ms2.merges.get(index).unwrap_or_else(|| {
                    Throwable::new(
                        "java.lang.IndexOutOfBoundsException",
                        format!(
                            "Index {index} out of bounds for length {}",
                            ms2.merges.len()
                        ),
                    )
                    .throw()
                });
                close_or_assertion_error(m);
                if let Err(e) = Thread::try_sleep(1) {
                    Throwable::from_cause("java.lang.AssertionError", e).throw();
                }
            }
        });
        t.start();
        Assert::assert_false(ms.r#await(10));
        stop.store(true, Ordering::SeqCst);
        let _ = t.join();
        for (j, m) in ms.merges.iter().enumerate() {
            if (j as i32) < i.load(Ordering::SeqCst) {
                Assert::assert_true(m.has_completed_successfully().unwrap_or_else(|| {
                    Throwable::new("java.util.NoSuchElementException", "No value present").throw()
                }));
            } else {
                Assert::assert_false(m.has_completed_successfully().is_some());
            }
        }
        Ok(())
    })
}

/// Port of `TestMergePolicy.testFinishTwice`.
fn test_finish_twice() -> Result<()> {
    with_dir(|dir| {
        let spec = create_random_merge_specification(&dir, 1)?;
        let one_merge = &spec.merges[0];
        one_merge.close(true, false, |_mr| Ok(()))?;
        expect_throws(ISE, || one_merge.close(false, false, |_mr| Ok(())));
        Ok(())
    })
}

/// Port of `TestMergePolicy.testTotalMaxDoc`.
fn test_total_max_doc() -> Result<()> {
    with_dir(|dir| {
        let spec = create_random_merge_specification(&dir, 1)?;
        let mut docs = 0;
        let one_merge = &spec.merges[0];
        for info in &one_merge.segments {
            docs += info.info.max_doc()?;
        }
        assert_equals(&docs, &one_merge.total_max_doc);
        Ok(())
    })
}

/// Port of `TestMergePolicy.createRandomMergeSpecification` (`private static`).
fn create_random_merge_specification(
    dir: &Arc<dyn Directory>,
    num_merges: i32,
) -> Result<MergeSpecification> {
    let mut ms = MergeSpecification::new();
    for _ in 0..num_merges {
        let name = TestUtil::random_simple_string(&*random()); // name
        let max_doc = random().next_int_bound(1000); // maxDoc
        let is_compound_file = random().next_boolean(); // isCompoundFile
        let id_string = TestUtil::random_simple_string_between(
            // id
            &*random(),
            ID_LENGTH as i32,
            ID_LENGTH as i32,
        );
        let mut id = [0u8; ID_LENGTH];
        id.copy_from_slice(id_string.as_bytes());
        let si = SegmentInfo::new_without_codec(
            Arc::clone(dir),       // dir
            Version::LATEST,       // version
            Some(Version::LATEST), // min version
            name,
            max_doc,
            is_compound_file,
            false,
            HashMap::new(), // diagnostics
            id,
            HashMap::new(),  // attributes
            Sort::default(), // indexSort
        )?;
        let segments: Vec<SegmentCommitInfo> = vec![SegmentCommitInfo::new(
            si,
            0,
            0,
            0,
            0,
            0,
            StringHelper::random_id(),
        )?];
        ms.add(OneMerge::new(segments)?);
    }
    Ok(ms)
}
