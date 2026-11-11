//! Port of `lucene/core/src/test/org/apache/lucene/util/TestCloseableThreadLocal.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::error::Result;
use rucene::util::CloseableThreadLocal;

use crate::core_support::{assert_equals, assert_null, class, test};

/// `TestCloseableThreadLocal.TEST_VALUE`.
pub const TEST_VALUE: &str = "initvaluetest";

/// `TestCloseableThreadLocal`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.util.TestCloseableThreadLocal",
        vec![
            test("testInitValue", test_init_value),
            test("testNullValue", test_null_value),
            test(
                "testDefaultValueWithoutSetting",
                test_default_value_without_setting,
            ),
        ],
    )
}

fn test_init_value() -> Result<()> {
    let tl = init_value_thread_local();
    let str = *tl.get().borrow();
    assert_equals(&Some(TEST_VALUE), &str);
    Ok(())
}

fn test_null_value() -> Result<()> {
    // Tests that null can be set as a valid value (LUCENE-1805). This
    // previously failed in get().
    let ctl: CloseableThreadLocal<Option<&'static str>> = CloseableThreadLocal::new(|| None);
    ctl.set(None);
    assert_null(&*ctl.get().borrow());
    Ok(())
}

fn test_default_value_without_setting() -> Result<()> {
    // LUCENE-1805: make sure default get returns null,
    // twice in a row
    let ctl: CloseableThreadLocal<Option<&'static str>> = CloseableThreadLocal::new(|| None);
    assert_null(&*ctl.get().borrow());
    Ok(())
}

/// `InitValueThreadLocal extends CloseableThreadLocal<Object>`, whose
/// `initialValue()` returns [`TEST_VALUE`].
pub fn init_value_thread_local() -> CloseableThreadLocal<Option<&'static str>> {
    CloseableThreadLocal::new(|| Some(TEST_VALUE))
}
