//! Port of `org.apache.lucene.internal.tests.FilterIndexInputAccess`.

#![deny(unsafe_code)]

use std::any::TypeId;
use std::fmt::Debug;

/// Access to filtering-`IndexInput` internals exposed to the test framework.
///
/// Equivalent to `org.apache.lucene.internal.tests.FilterIndexInputAccess`.
///
/// Lucene's `FilterIndexInput.unwrapOnlyTest(IndexInput)` peels off wrappers
/// that only exist in tests before deciding whether an input can be scored
/// directly off its mapped bytes. The test framework registers those wrapper
/// classes here.
///
/// # Divergences from Lucene 10.5.0
///
/// * **`TypeId` instead of `Class<? extends FilterIndexInput>`.** Rust has no
///   class objects; [`TypeId`] is the run-time type identity it does have. The
///   bound Java expresses in the signature cannot be expressed on a `TypeId`,
///   so callers are expected to register a type implementing
///   [`IndexInput`](crate::store::IndexInput); the convenience
///   [`add_test_filter_type_of`](Self::add_test_filter_type_of) states that
///   bound at compile time.
/// * **No `FilterIndexInput` in this port.** Rucene has not ported
///   `org.apache.lucene.store.FilterIndexInput` yet, so the registry has no
///   consumer inside the crate; the interface is ported because it is part of
///   this package, and it will bind to that type once it exists.
/// * **`Send + Sync + Debug` bounds.** Needed because
///   [`TestSecrets`](super::TestSecrets) keeps the accessor in a `static`.
pub trait FilterIndexInputAccess: Send + Sync + Debug {
    /// Adds the given test filtering-input type.
    ///
    /// Equivalent to
    /// `FilterIndexInputAccess.addTestFilterType(Class<? extends FilterIndexInput>)`.
    fn add_test_filter_type(&self, type_id: TypeId);

    /// Adds the given test filtering-input type, naming it at compile time.
    ///
    /// Convenience wrapper over [`add_test_filter_type`](Self::add_test_filter_type)
    /// that restores the `Class<? extends FilterIndexInput>` bound Java states
    /// in the signature. It has no counterpart in Lucene.
    fn add_test_filter_type_of<T: crate::store::IndexInput + 'static>(&self)
    where
        Self: Sized,
    {
        self.add_test_filter_type(TypeId::of::<T>());
    }
}
