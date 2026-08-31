//! Port of `org.apache.lucene.util.hnsw.HasKnnVectorValues`.

use crate::index::vector_values::KnnVectorValues;

/// Implementors can return the [`KnnVectorValues`] from which their scorers read.
///
/// Equivalent to `org.apache.lucene.util.hnsw.HasKnnVectorValues`.
pub trait HasKnnVectorValues {
    /// Returns the backing vector values, or `None`.
    fn values(&self) -> Option<&dyn KnnVectorValues>;
}
