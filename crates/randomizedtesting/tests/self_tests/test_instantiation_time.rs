//! Port of `com.carrotsearch.randomizedtesting.TestInstantiationTime`
//! (randomized-runner 2.8.1 test sources, Apache-2.0): `@BeforeClass` hooks
//! run before instance initializers.

use std::sync::{Arc, Mutex};

use randomizedtesting::jdk::exception::{self, NULL_POINTER_EXCEPTION};
use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::BeforeClass;

use crate::support::{cls, static_hook, test, RANDOMIZED_TEST};

/// `private static String constant`.
static CONSTANT: Mutex<Option<String>> = Mutex::new(None);

/// The instance: `public String copyOfStatic = constant.toUpperCase();`.
struct Instance {
    #[allow(dead_code)]
    copy_of_static: String,
}

/// `TestInstantiationTime extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestInstantiationTime")
        .extends(&RANDOMIZED_TEST)
        .constructor(Constructor::no_args(|| {
            // An NPE if prepare() was not invoked before.
            let c = CONSTANT.lock().unwrap_or_else(|e| e.into_inner()).clone();
            match c {
                Some(c) => Instance {
                    copy_of_static: c.to_uppercase(),
                },
                None => exception::throw_without_message(NULL_POINTER_EXCEPTION),
            }
        }))
        .method(static_hook("prepare", BeforeClass, || {
            *CONSTANT.lock().unwrap_or_else(|e| e.into_inner()) = Some("constant".into());
        }))
        .method(test("testDummy", || {}))
        .build()]
}
