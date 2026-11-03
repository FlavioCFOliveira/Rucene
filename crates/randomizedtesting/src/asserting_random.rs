//! Port of `com.carrotsearch.randomizedtesting.AssertingRandom`
//! (randomizedtesting 2.8.1, `AssertingRandom.java`).

use std::backtrace::Backtrace;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::jdk::exception::{self, ILLEGAL_STATE_EXCEPTION};
use crate::jdk::random::{Random, RandomFields};
use crate::jdk::thread::{Thread, WeakThread};
use crate::stack_trace_holder::StackTraceHolder;

/// A `Random` with a delegate, preventing `setSeed` and locked to the single
/// thread that owns it: `AssertingRandom extends java.util.Random`.
///
/// Every overridden method first checks that the instance is still valid and
/// is called from its owner thread, then delegates. The methods it does not
/// override (`nextInt(int, int)`, `nextLong(long)`, the streams, ...) are
/// `java.util.Random`'s, which reach the delegate through the overridden
/// ones; `next(int)` is unreachable.
///
/// **Divergences.** `toString`, `equals` and `hashCode` (which Java
/// checks and delegates) are `Object` identity methods with no Rust
/// counterpart on a trait object; `clone` (which Java refuses with
/// `CloneNotSupportedException`) is refused at compile time, since the type
/// does not implement `Clone`.
pub struct AssertingRandom {
    fields: RandomFields,
    delegate: Arc<dyn Random>,
    owner_ref: WeakThread,
    owner_name: String,
    allocation_stack: Arc<Backtrace>,
    /// Java's `volatile boolean valid`.
    valid: AtomicBool,
}

impl AssertingRandom {
    /// `new AssertingRandom(owner, delegate)`: usable by `owner` until
    /// [`destroy`](Self::destroy)ed.
    pub fn new(owner: &Thread, delegate: Arc<dyn Random>) -> AssertingRandom {
        AssertingRandom {
            // `super(0)`.
            fields: RandomFields::new(0),
            delegate,
            owner_ref: owner.downgrade(),
            owner_name: owner.to_string(),
            allocation_stack: Arc::new(Backtrace::capture()),
            valid: AtomicBool::new(true),
        }
    }

    /// `destroy()`: the instance is no longer usable after this call.
    pub fn destroy(&self) {
        // Release pairs with the Acquire load of `check_valid`, as Java's
        // volatile write and read do.
        self.valid.store(false, Ordering::Release);
    }

    /// `isVerifying()`: whether sharing and lifecycle assertions are on.
    ///
    /// **Divergence.** Java answers
    /// `AssertingRandom.class.desiredAssertionStatus()` (`-ea`); the port
    /// answers `cfg!(debug_assertions)`, the Rust counterpart.
    pub fn is_verifying() -> bool {
        cfg!(debug_assertions)
    }

    /// `AssertingRandom.noSetSeed()`: the refusal of `setSeed`, thrown.
    ///
    /// # Panics
    ///
    /// Always: `RuntimeException` with Java's message.
    pub(crate) fn no_set_seed() -> ! {
        exception::runtime(
            "RandomizedRunner prevents changing the seed of its random generators to assure \
             repeatability of tests. If you need a mutable instance of Random, create a new \
             (local) instance, preferably with the initial seed aquired from this Random \
             instance.",
        )
    }

    /// `checkValid()`.
    fn check_valid(&self) {
        if !AssertingRandom::is_verifying() {
            return;
        }
        if !self.valid.load(Ordering::Acquire) {
            exception::illegal_state(
                "This Random instance has been invalidated and is probably used out of its \
                 allowed context (test or suite).",
            );
        }
        let current = Thread::current();
        let is_owner = self.owner_ref.get().is_some_and(|owner| owner == current);
        if !is_owner {
            let cause = StackTraceHolder::new(
                format!(
                    "Original allocation stack for this Random (allocated by {})",
                    self.owner_name
                ),
                Arc::clone(&self.allocation_stack),
            );
            // Java chains the holder as the cause; the panic message carries
            // it after the exception, as a printed stack trace would.
            panic!(
                "{}\nCaused by: {}",
                exception::to_string(
                    ILLEGAL_STATE_EXCEPTION,
                    Some(&format!(
                        "This Random was created for/by another thread ({}). Random instances \
                         must not be shared (acquire per-thread). Current thread: {}",
                        self.owner_name, current
                    ))
                ),
                cause
            );
        }
    }
}

impl Random for AssertingRandom {
    fn fields(&self) -> &RandomFields {
        &self.fields
    }

    /// `next(bits)`: unreachable by design.
    ///
    /// # Panics
    ///
    /// Always: `RuntimeException` (`Shouldn't be reachable.`).
    fn next(&self, _bits: i32) -> i32 {
        exception::runtime("Shouldn't be reachable.")
    }

    fn next_boolean(&self) -> bool {
        self.check_valid();
        self.delegate.next_boolean()
    }

    fn next_bytes(&self, bytes: &mut [u8]) {
        self.check_valid();
        self.delegate.next_bytes(bytes)
    }

    fn next_double(&self) -> f64 {
        self.check_valid();
        self.delegate.next_double()
    }

    fn next_float(&self) -> f32 {
        self.check_valid();
        self.delegate.next_float()
    }

    fn next_gaussian(&self) -> f64 {
        self.check_valid();
        self.delegate.next_gaussian()
    }

    fn next_int(&self) -> i32 {
        self.check_valid();
        self.delegate.next_int()
    }

    fn next_int_bound(&self, bound: i32) -> i32 {
        self.check_valid();
        self.delegate.next_int_bound(bound)
    }

    fn next_long(&self) -> i64 {
        self.check_valid();
        self.delegate.next_long()
    }

    /// `setSeed(seed)`: always refused once constructed (Java returns only
    /// for the call its superclass constructor makes before `delegate` is
    /// set, which Rust does not make).
    ///
    /// # Panics
    ///
    /// `RuntimeException` with `noSetSeed()`'s message.
    fn set_seed(&self, _seed: i64) {
        AssertingRandom::no_set_seed()
    }
}
