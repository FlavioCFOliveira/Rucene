//! Port of `com.carrotsearch.randomizedtesting.RandomSupplier`
//! (randomizedtesting 2.8.1, `RandomSupplier.java`).

use std::sync::Arc;

use crate::jdk::random::Random;
use crate::xoroshiro128_plus_random::Xoroshiro128PlusRandom;

/// A factory of the `Random` behind each `Randomness`: `RandomSupplier`.
pub trait RandomSupplier: Send + Sync {
    /// `get(long seed)`.
    fn get(&self, seed: i64) -> Arc<dyn Random>;
}

/// `RandomSupplier.DEFAULT`: a [`Xoroshiro128PlusRandom`] for each seed.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultRandomSupplier;

impl RandomSupplier for DefaultRandomSupplier {
    fn get(&self, seed: i64) -> Arc<dyn Random> {
        Arc::new(Xoroshiro128PlusRandom::new(seed))
    }
}

/// `RandomSupplier.DEFAULT`.
pub const DEFAULT: DefaultRandomSupplier = DefaultRandomSupplier;

/// `RandomSupplier.DEFAULT`, shared.
pub fn default_supplier() -> Arc<dyn RandomSupplier> {
    Arc::new(DEFAULT)
}
