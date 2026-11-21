//! Port of `org.apache.lucene.util.hnsw.IntToIntFunction`.

/// Native int-to-int function.
///
/// Equivalent to `org.apache.lucene.util.hnsw.IntToIntFunction`.
pub trait IntToIntFunction {
    /// Applies this function to `v`.
    fn apply(&self, v: i32) -> i32;
}

impl<F: Fn(i32) -> i32> IntToIntFunction for F {
    fn apply(&self, v: i32) -> i32 {
        self(v)
    }
}
