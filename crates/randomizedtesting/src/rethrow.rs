//! Port of `com.carrotsearch.randomizedtesting.Rethrow`
//! (randomizedtesting 2.8.1, `Rethrow.java`).

use std::any::Any;

/// Rethrowing checked exceptions as unchecked ones: `Rethrow`.
pub struct Rethrow;

impl Rethrow {
    /// `rethrow(Throwable t)`: rethrows `t`, the identical object.
    ///
    /// **Divergence.** Java rethrows any `Throwable` past the
    /// compiler's checked-exception analysis and declares a
    /// `RuntimeException` return it never produces; the Rust counterpart of
    /// a thrown object is a panic payload, which this resumes unchanged
    /// (`std::panic::resume_unwind`), and it never returns.
    pub fn rethrow(t: Box<dyn Any + Send>) -> ! {
        std::panic::resume_unwind(t)
    }
}
