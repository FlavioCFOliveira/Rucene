//! Port of `com.carrotsearch.randomizedtesting.generators.TestRealisticUnicodeGenerator`
//! (randomized-runner 2.8.1 test sources, Apache-2.0).

use std::sync::Arc;

use randomizedtesting::generators::RealisticUnicodeGenerator;
use randomizedtesting::jdk::reflect::Class;

use super::cls;
use super::string_generator_test_base::{constructor, identity, CLASS as BASE};

/// `TestRealisticUnicodeGenerator extends StringGeneratorTestBase`.
pub fn classes() -> Vec<Arc<Class>> {
    vec![cls("TestRealisticUnicodeGenerator")
        .extends(&BASE)
        .constructor(constructor(
            || Arc::new(RealisticUnicodeGenerator::new()),
            identity,
        ))
        .build()]
}
