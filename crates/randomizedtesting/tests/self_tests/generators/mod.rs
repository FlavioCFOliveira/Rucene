//! Package `com.carrotsearch.randomizedtesting.generators` of the runner's
//! own tests.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;

mod string_generator_test_base;
mod test_codepoint_set_generator;
mod test_random_bytes;
mod test_random_picks;
mod test_realistic_unicode_generator;
mod test_unicode_generator;

/// `generators.<TestClass>` entries.
pub fn entries() -> Vec<crate::Entry> {
    vec![
        (
            "generators.TestCodepointSetGenerator",
            test_codepoint_set_generator::classes as fn() -> Vec<Arc<Class>>,
        ),
        ("generators.TestRandomBytes", test_random_bytes::classes),
        ("generators.TestRandomPicks", test_random_picks::classes),
        (
            "generators.TestRealisticUnicodeGenerator",
            test_realistic_unicode_generator::classes,
        ),
        (
            "generators.TestUnicodeGenerator",
            test_unicode_generator::classes,
        ),
    ]
}

/// A builder for `com.carrotsearch.randomizedtesting.generators.<name>`.
pub fn cls(name: &str) -> randomizedtesting::jdk::reflect::ClassBuilder {
    crate::support::cls(&format!("generators.{name}"))
}
