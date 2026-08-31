//! Port of `org.apache.lucene.internal.vectorization.DefaultVectorizationProvider`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::hnsw::{DefaultFlatVectorScorer, FlatVectorsScorer};
use crate::codecs::lucene99::scalar_quantized_scorer::Lucene99ScalarQuantizedVectorScorer;
use crate::error::Result;
use crate::internal::vectorization::{
    DefaultDocValuesBulkDecodeSupport, DefaultVectorUtilSupport, DocValuesBulkDecodeSupport,
    PostingDecodingUtil, VectorUtilSupport, VectorizationProvider,
};
use crate::store::IndexInput;

/// Default provider returning scalar implementations.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.DefaultVectorizationProvider`.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene declares this class package-private. Rust has no package visibility,
/// and [`lookup`](super::lookup) must be able to name the type it returns, so
/// it is `pub` here. It is not part of Rucene's supported API.
#[derive(Debug)]
pub struct DefaultVectorizationProvider {
    vector_util_support: DefaultVectorUtilSupport,
    /// Java hands out the `DefaultFlatVectorScorer.INSTANCE` singleton; the
    /// port keeps one shared handle so that every call returns the same object.
    lucene99_flat_vectors_scorer: Arc<dyn FlatVectorsScorer>,
}

impl DefaultVectorizationProvider {
    /// Creates the provider.
    ///
    /// Equivalent to the package-private `DefaultVectorizationProvider()`
    /// constructor, which allocates the scalar `VectorUtilSupport`.
    pub fn new() -> Self {
        Self {
            vector_util_support: DefaultVectorUtilSupport::new(),
            lucene99_flat_vectors_scorer: Arc::new(DefaultFlatVectorScorer::INSTANCE),
        }
    }
}

impl Default for DefaultVectorizationProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl VectorizationProvider for DefaultVectorizationProvider {
    fn get_vector_util_support(&self) -> &dyn VectorUtilSupport {
        &self.vector_util_support
    }

    fn get_lucene99_flat_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer> {
        Arc::clone(&self.lucene99_flat_vectors_scorer)
    }

    fn get_lucene99_scalar_quantized_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer> {
        Arc::new(Lucene99ScalarQuantizedVectorScorer::new(Arc::clone(
            &self.lucene99_flat_vectors_scorer,
        )))
    }

    fn new_posting_decoding_util(&self, input: Box<dyn IndexInput>) -> Result<PostingDecodingUtil> {
        Ok(PostingDecodingUtil::new(input))
    }

    fn get_doc_values_bulk_decode_support(&self) -> &'static dyn DocValuesBulkDecodeSupport {
        static DEFAULT: DefaultDocValuesBulkDecodeSupport =
            DefaultDocValuesBulkDecodeSupport::INSTANCE;
        &DEFAULT
    }
}
