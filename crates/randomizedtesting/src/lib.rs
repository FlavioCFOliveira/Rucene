//! Port of randomizedtesting 2.8.1 (`com.carrotsearch.randomizedtesting`,
//! module `randomized-runner`, tag `release/2.8.1`), the runner Apache
//! Lucene's tests run under.
//!
//! Every test's randomness derives from one master seed: the runner seeds a
//! [`Randomness`] per suite and per test, [`RandomizedContext`] hands each
//! thread its own [`jdk::random::Random`], and [`RandomizedTest`] and the
//! [`generators`] draw numbers, strings, bytes and picks from it. With the
//! same seed a Rust test and a Java test draw the same values.
//!
//! The Java package maps to this crate's root, its `generators` and
//! `annotations` packages to the modules of the same names, and the JDK
//! behaviour it relies on to [`jdk`].
//!
//! Licensed under the Apache License 2.0, as randomizedtesting is; see the
//! crate's `NOTICE`.

pub mod annotations;
pub mod generators;
pub mod jdk;

mod asserting_random;
mod closeable_resource_info;
mod group_evaluator;
mod lifecycle_scope;
mod mix_with_suite_name;
mod murmur_hash3;
mod object_procedure;
mod random_supplier;
mod randomized_context;
mod randomized_test;
mod randomness;
mod resource_disposal_error;
mod rethrow;
mod seed_decorator;
mod seed_utils;
mod stack_trace_holder;
mod sys_globals;
mod temp_path_resource;
mod threads;
mod xoroshiro128_plus_random;

pub use asserting_random::AssertingRandom;
pub use group_evaluator::GroupEvaluator;
pub use lifecycle_scope::LifecycleScope;
pub use mix_with_suite_name::MixWithSuiteName;
pub use random_supplier::{default_supplier, DefaultRandomSupplier, RandomSupplier, DEFAULT};
pub use randomized_context::RandomizedContext;
pub use randomized_test::{RandomizedTest, ServerSocket, ASSUMPTION_VIOLATED_EXCEPTION};
pub use randomness::Randomness;
pub use rethrow::Rethrow;
pub use seed_decorator::SeedDecorator;
pub use seed_utils::SeedUtils;
pub use sys_globals::{SysGlobals, CHILDVM_SYSPROP_JVM_COUNT, CHILDVM_SYSPROP_JVM_ID};
pub use temp_path_resource::TempPathResource;
pub use xoroshiro128_plus_random::Xoroshiro128PlusRandom;

#[cfg(test)]
mod tests;
