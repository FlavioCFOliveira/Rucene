//! Port of `lucene/core/src/test/org/apache/lucene/util/TestSetOnce.java`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::thread::Thread;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::util::SetOnce;
use rucene_test_framework::util::java_exceptions::expect_throws;

use crate::core_support::{assert_equals, assert_equals_msg, assert_null, class, random, test};

/// `SetOnce.AlreadySetException`: Rucene raises it as
/// `LuceneError::IllegalState` (its Java superclass), so the port expects
/// `IllegalStateException`.
const ALREADY_SET: &str = "java.lang.IllegalStateException";

/// `TestSetOnce`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestSetOnce",
        vec![
            test("testEmptyCtor", test_empty_ctor),
            test("testSettingCtor", test_setting_ctor),
            test("testSetOnce", test_set_once),
            test("testTrySet", test_try_set),
            test("testSetMultiThreaded", test_set_multi_threaded),
        ],
    )
}

/// `set.get().intValue()`: a `NullPointerException` when unset.
fn int_value(set: &SetOnce<i32>) -> i32 {
    match set.get() {
        Some(v) => *v,
        None => Throwable::new("java.lang.NullPointerException", "null".to_string()).throw(),
    }
}

fn test_empty_ctor() -> Result<()> {
    let set: SetOnce<i32> = SetOnce::new();
    assert_null(&set.get());
    Ok(())
}

fn test_setting_ctor() -> Result<()> {
    let set = SetOnce::with_value(5);
    assert_equals(&5, &int_value(&set));
    expect_throws(ALREADY_SET, || set.set(7));
    Ok(())
}

fn test_set_once() -> Result<()> {
    let set = SetOnce::new();
    set.set(5)?;
    assert_equals(&5, &int_value(&set));
    expect_throws(ALREADY_SET, || set.set(7));
    Ok(())
}

fn test_try_set() -> Result<()> {
    let set = SetOnce::new();
    Assert::assert_true(set.try_set(5));
    assert_equals(&5, &int_value(&set));
    Assert::assert_false(set.try_set(7));
    assert_equals(&5, &int_value(&set));
    Ok(())
}

/// `SetOnceThread`: its `success` flag beside the thread.
struct SetOnceThread {
    thread: Thread,
    success: Arc<AtomicBool>,
}

fn test_set_multi_threaded() -> Result<()> {
    let set: Arc<SetOnce<i32>> = Arc::new(SetOnce::new());
    let mut threads: Vec<SetOnceThread> = Vec::with_capacity(10);
    for i in 0..10 {
        // `new SetOnceThread(random())`: RAND = new Random(random.nextLong()).
        let rand = JdkRandom::new(random().next_long());
        let name = format!("t-{}", i + 1);
        let success = Arc::new(AtomicBool::new(false));
        let thread_set = Arc::clone(&set);
        let thread_success = Arc::clone(&success);
        let thread_name = name.clone();
        let thread = Thread::new(name, move || {
            // sleep for a short time
            if Thread::try_sleep(rand.next_int_bound(10) as i64).is_err() {
                // InterruptedException: ignored
                return;
            }
            let value: i32 = thread_name[2..].parse().unwrap_or_else(|_| {
                Throwable::new("java.lang.NumberFormatException", thread_name.clone()).throw()
            });
            match thread_set.set(value) {
                Ok(()) => thread_success.store(true, Ordering::SeqCst),
                // RuntimeException
                Err(_) => thread_success.store(false, Ordering::SeqCst),
            }
        });
        threads.push(SetOnceThread { thread, success });
    }

    for t in &threads {
        t.thread.start();
    }

    for t in &threads {
        let _ = t.thread.join();
    }

    for t in &threads {
        if t.success.load(Ordering::SeqCst) {
            let name = t.thread.name();
            let expected_val: i32 = name[2..].parse().unwrap_or_else(|_| {
                Throwable::new("java.lang.NumberFormatException", name.clone()).throw()
            });
            assert_equals_msg(
                Some(&format!("thread {name}")),
                &expected_val,
                &int_value(&set),
            );
        }
    }
    Ok(())
}
