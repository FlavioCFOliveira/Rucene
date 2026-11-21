//! Port of `com.carrotsearch.randomizedtesting.generators.TestUnicodeGenerator`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::generators::UnicodeGenerator;
use randomizedtesting::jdk::reflect::Class;

use super::cls;
use super::string_generator_test_base::{constructor, identity, CLASS as BASE};

/// `TestUnicodeGenerator extends StringGeneratorTestBase`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestUnicodeGenerator")
        .extends(&BASE)
        .constructor(constructor(|| Arc::new(UnicodeGenerator::new()), identity))
        .build()]
}
