//! Port of `com.carrotsearch.randomizedtesting.Randomness`
//! (randomizedtesting 2.8.1, `Randomness.java`).

use std::fmt;
use std::sync::Arc;

use crate::asserting_random::AssertingRandom;
use crate::jdk::random::Random;
use crate::jdk::thread::Thread;
use crate::random_supplier::RandomSupplier;
use crate::seed_decorator::SeedDecorator;
use crate::seed_utils::SeedUtils;

/// `SeedDecorator[]`, shared between a randomness and its clones.
type SeedDecorators = Arc<[Arc<dyn SeedDecorator>]>;

/// Per-thread, per-lifecycle randomness: an initial seed and the `Random`
/// built from it (`Randomness`). Typically obtained from a
/// `RandomizedContext`.
///
/// The `Random` is built from the seed after every decorator in turn
/// decorated it; `get_seed` is the undecorated seed. With
/// assertions on ([`AssertingRandom::is_verifying`]) the `Random` is an
/// [`AssertingRandom`] owned by the given thread.
///
/// `Clone` copies the handle (the same seed and the same `Random`
/// instance), as passing a Java reference does; Java's
/// `Randomness.clone(Thread)` is `clone_for`.
#[derive(Clone)]
pub struct Randomness {
    seed: i64,
    random: Arc<dyn Random>,
    asserting: Option<Arc<AssertingRandom>>,
    supplier: Arc<dyn RandomSupplier>,
    decorators: SeedDecorators,
}

impl Randomness {
    /// `new Randomness(Thread owner, RandomSupplier supplier, long seed,
    /// SeedDecorator... decorators)`.
    pub fn with_owner(
        owner: &Thread,
        supplier: Arc<dyn RandomSupplier>,
        seed: i64,
        decorators: Vec<Arc<dyn SeedDecorator>>,
    ) -> Randomness {
        Randomness::build(owner, supplier, seed, Arc::from(decorators))
    }

    /// `new Randomness(long seed, RandomSupplier supplier, SeedDecorator...
    /// decorators)`, owned by the current thread.
    pub fn new(
        seed: i64,
        supplier: Arc<dyn RandomSupplier>,
        decorators: Vec<Arc<dyn SeedDecorator>>,
    ) -> Randomness {
        Randomness::with_owner(&Thread::current(), supplier, seed, decorators)
    }

    fn build(
        owner: &Thread,
        supplier: Arc<dyn RandomSupplier>,
        seed: i64,
        decorators: SeedDecorators,
    ) -> Randomness {
        let delegate = supplier.get(Randomness::decorate(seed, &decorators));
        let (random, asserting): (Arc<dyn Random>, _) = if AssertingRandom::is_verifying() {
            let a = Arc::new(AssertingRandom::new(owner, delegate));
            (a.clone(), Some(a))
        } else {
            (delegate, None)
        };
        Randomness {
            seed,
            random,
            asserting,
            supplier,
            decorators,
        }
    }

    /// `getRandom()`: the `Random` of this randomness.
    pub fn get_random(&self) -> Arc<dyn Random> {
        Arc::clone(&self.random)
    }

    /// `getRandomSupplier()` (package-private).
    pub(crate) fn get_random_supplier(&self) -> Arc<dyn RandomSupplier> {
        Arc::clone(&self.supplier)
    }

    /// `getDecorators()` (package-private).
    pub(crate) fn get_decorators(&self) -> Vec<Arc<dyn SeedDecorator>> {
        self.decorators.to_vec()
    }

    /// `clone(Thread newOwner)` (package-private): a new randomness with the
    /// same seed, supplier and decorators, owned by `new_owner`.
    pub(crate) fn clone_for(&self, new_owner: &Thread) -> Randomness {
        Randomness::build(
            new_owner,
            Arc::clone(&self.supplier),
            self.seed,
            Arc::clone(&self.decorators),
        )
    }

    /// `destroy()` (package-private): invalidates the underlying
    /// `AssertingRandom`, if any.
    pub(crate) fn destroy(&self) {
        if let Some(a) = &self.asserting {
            a.destroy();
        }
    }

    /// `getSeed()` ("read-only for tests"): the starting seed, undecorated.
    ///
    /// Package-private in Java, where the runner's own tests reach it from the
    /// same package; public here because those tests, ported, are an integration
    /// test target outside the crate.
    pub fn get_seed(&self) -> i64 {
        self.seed
    }

    /// `decorate(long, SeedDecorator[])`.
    fn decorate(seed: i64, decorators: &[Arc<dyn SeedDecorator>]) -> i64 {
        decorators.iter().fold(seed, |s, d| d.decorate(s))
    }
}

impl fmt::Display for Randomness {
    /// `toString()`: `[Randomness, seed=[S]]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "[Randomness, seed={}]",
            SeedUtils::format_seed_chain(&[self])
        )
    }
}

impl fmt::Debug for Randomness {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}
