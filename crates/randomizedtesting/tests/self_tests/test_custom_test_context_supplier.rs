//! Port of `com.carrotsearch.randomizedtesting.TestCustomTestContextSupplier`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use randomizedtesting::annotations::{ClassRef, TestContextRandomSupplier};
use randomizedtesting::jdk::random::{Random, RandomFields};
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::jdk::system;
use randomizedtesting::{RandomSupplier, RandomizedContext};

use crate::support::{assert_equals, cls, test, unit, RANDOMIZED_TEST};

/// The anonymous `new Random() { int value; nextInt() { return value++; } }`.
struct Counting {
    fields: RandomFields,
    value: AtomicI32,
}

impl Random for Counting {
    fn fields(&self) -> &RandomFields {
        &self.fields
    }
    fn next_int(&self) -> i32 {
        self.value.fetch_add(1, Ordering::SeqCst)
    }
}

/// `TestCustomTestContextSupplier.CustomSupplier implements RandomSupplier`.
struct CustomSupplier;

impl RandomSupplier for CustomSupplier {
    fn get(&self, _seed: i64) -> Arc<dyn Random> {
        Arc::new(Counting {
            fields: RandomFields::new(system::nano_time()),
            value: AtomicI32::new(0),
        })
    }
}

/// `@TestContextRandomSupplier(CustomSupplier.class) TestCustomTestContextSupplier extends RandomizedTest`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestCustomTestContextSupplier")
        .extends(&RANDOMIZED_TEST)
        .annotate(TestContextRandomSupplier {
            value: ClassRef::new(
                "com.carrotsearch.randomizedtesting.TestCustomTestContextSupplier$CustomSupplier",
                || Arc::new(CustomSupplier) as Arc<dyn RandomSupplier>,
            ),
        })
        .constructor(unit())
        .method(test("testCustomSupplier", || {
            // Can't just check the type because random can be wrapped with AssertingRandom.
            let rnd = RandomizedContext::current().get_random();
            for i in 0..10 {
                assert_equals(rnd.next_int(), i);
            }
        }))
        .build()]
}
