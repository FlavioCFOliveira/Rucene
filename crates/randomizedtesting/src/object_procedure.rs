//! Port of `com.carrotsearch.randomizedtesting.ObjectProcedure`
//! (randomizedtesting 2.8.1, `ObjectProcedure.java`).

/// A procedure applied to each value: `ObjectProcedure<KType>`
/// (package-private). Any `FnMut(K)` is one.
#[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
pub(crate) trait ObjectProcedure<K> {
    /// `apply(KType value)`.
    fn apply(&mut self, value: K);
}

impl<K, F: FnMut(K)> ObjectProcedure<K> for F {
    fn apply(&mut self, value: K) {
        self(value)
    }
}
