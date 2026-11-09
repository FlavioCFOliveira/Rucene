//! The members a `Test…SuiteDraws` class of `BaseSuitesDrawsOracle` adds to
//! the suite it extends (rmp #350, #356, #358): a `tearDown()` that records
//! the next `random().nextLong()` of the test's randomness before calling
//! `super.tearDown()`, and an `@AfterClass` method that records that of the
//! class randomness.

use std::any::Any;
use std::sync::Mutex;

use randomizedtesting::jdk::reflect::{modifier, Method};
use rucene_test_framework::util::LuceneTestCase;

/// `tearDown()`: records `<method>\tnext\t<long>` into `draws`, then runs
/// `super_tear_down` (the `tearDown()` of the class the draws class
/// extends).
pub fn record_tear_down<T: Any + Send>(
    draws: &'static Mutex<Vec<String>>,
    super_tear_down: fn(&mut T),
) -> Method {
    Method::new("tearDown")
        .throws(&["java.lang.Exception"])
        .body(move |inst: &mut T| {
            draws.lock().unwrap().push(format!(
                "{}\tnext\t{}",
                LuceneTestCase::get_test_name().unwrap_or_default(),
                LuceneTestCase::random().next_long()
            ));
            super_tear_down(inst);
        })
}

/// `LuceneTestCase.tearDown()`, the `super_tear_down` of a suite that does
/// not override it.
pub fn lucene_tear_down<T>(_inst: &mut T) {
    LuceneTestCase::super_tear_down();
}

/// `@AfterClass public static void recordClassDraw()`: records
/// `#class\tnext\t<long>` into `draws`.
pub fn record_class_draw(draws: &'static Mutex<Vec<String>>) -> Method {
    Method::new("recordClassDraw")
        .modifiers(modifier::PUBLIC | modifier::STATIC)
        .annotate(randomizedtesting::junit::AfterClass)
        .static_body(move || {
            draws.lock().unwrap().push(format!(
                "#class\tnext\t{}",
                LuceneTestCase::random().next_long()
            ));
        })
}
