//! Port of `org.apache.lucene.internal.vectorization.PanamaVectorizationProvider`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::hnsw::FlatVectorsScorer;
use crate::error::{LuceneError, Result};
use crate::internal::vectorization::{
    DocValuesBulkDecodeSupport, DocValuesRangeSupport, Lucene99MemorySegmentFlatVectorsScorer,
    Lucene99MemorySegmentScalarQuantizedVectorScorer, PanamaDocValuesBulkDecodeSupport,
    PanamaDocValuesRangeSupport, PanamaVectorConstants, PanamaVectorUtilSupport,
    PostingDecodingUtil, VectorUtilSupport, VectorizationProvider,
};
use crate::store::IndexInput;

/// A vectorization provider that leverages the Panama Vector API.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaVectorizationProvider`.
///
/// It is the counterpart of
/// [`DefaultVectorizationProvider`](super::DefaultVectorizationProvider): the
/// same six accessors, wired to the memory-segment scorers and the Panama
/// support classes instead of the scalar ones.
///
/// # Reachability
///
/// [`lookup`](super::lookup) selects this provider by Lucene's own gates
/// (`VectorizationProvider.lookup`); see there for which of them pass in a
/// Rust binary.
///
/// # Divergence from Lucene 10.5.0, declared
///
/// **No JDK-8309727 warm-up.** Java loads one `FloatVector` inside
/// `AccessController.doPrivileged` to work around a JDK initialisation bug and
/// turns its `SecurityException` into `UnsupportedOperationException`. There
/// is no Vector API class loading and no security manager here, so the
/// warm-up has nothing to do and cannot fail.
#[derive(Debug)]
pub struct PanamaVectorizationProvider {
    vector_util_support: PanamaVectorUtilSupport,
}

impl PanamaVectorizationProvider {
    /// Runs `action` with the privileges Lucene asks the access controller for.
    ///
    /// Equivalent to
    /// `org.apache.lucene.internal.vectorization.PanamaVectorizationProvider#doPrivileged`,
    /// which exists in Java only so that the `@SuppressForbidden` annotation
    /// can be applied to a single method: it wraps its argument in
    /// `AccessController.doPrivileged` so the JVM's `SecurityManager` grants
    /// the vector load of the JDK-8309727 warm-up. Rust has neither an access
    /// controller nor a security manager — the caller's privileges *are* the
    /// process's privileges — so the privileged block is the action itself,
    /// exactly as
    /// [`Constants::do_privileged`](crate::util::Constants) resolves the same
    /// construct elsewhere in this crate.
    ///
    /// # Divergence from Lucene 10.5.0
    ///
    /// Lucene declares the helper `private static` and has one call site, the
    /// JDK-8309727 warm-up that the constructor of this port does not reproduce
    /// (see the type's divergences). The helper therefore has no caller here
    /// and is `pub` rather than private.
    pub fn do_privileged<T>(action: impl FnOnce() -> T) -> T {
        action()
    }

    /// Creates the provider.
    ///
    /// Equivalent to the package-private `PanamaVectorizationProvider()`
    /// constructor (`PanamaVectorizationProvider.java:62-87`).
    ///
    /// # Errors
    ///
    /// Returns Java's `UnsupportedOperationException("Vector bit size is less
    /// than 128: " + PREFERRED_VECTOR_BITSIZE)` when
    /// [`PanamaVectorConstants::preferred_vector_bitsize`] is below 128
    /// (`PanamaVectorizationProvider.java:75`); [`lookup`](super::lookup)
    /// then falls back to the scalar provider, as Lucene's does.
    pub fn new() -> Result<Self> {
        let bits = PanamaVectorConstants::preferred_vector_bitsize();
        if bits < 128 {
            return Err(LuceneError::unsupported_operation(format!(
                "Vector bit size is less than 128: {bits}"
            )));
        }
        log::info!(
            "Java vector incubator API enabled; uses preferredBitSize={}{}{}",
            bits,
            if super::panama_vector_util_support::use_vector_fma() {
                "; FMA enabled"
            } else {
                ""
            },
            if PanamaVectorConstants::has_fast_integer_vectors() {
                ""
            } else {
                "; floating-point vectors only"
            }
        );
        Ok(Self {
            vector_util_support: PanamaVectorUtilSupport::new(),
        })
    }
}

impl VectorizationProvider for PanamaVectorizationProvider {
    fn get_vector_util_support(&self) -> &dyn VectorUtilSupport {
        &self.vector_util_support
    }

    fn get_lucene99_flat_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer> {
        Lucene99MemorySegmentFlatVectorsScorer::instance()
    }

    fn get_lucene99_scalar_quantized_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer> {
        Arc::new(Lucene99MemorySegmentScalarQuantizedVectorScorer::INSTANCE)
    }

    fn new_posting_decoding_util(&self, input: Box<dyn IndexInput>) -> Result<PostingDecodingUtil> {
        // Lucene additionally tests `HAS_FAST_INTEGER_VECTORS && input
        // instanceof MemorySegmentAccessInput` and, when the whole file fits in
        // one segment, returns a `MemorySegmentPostingDecodingUtil`. The
        // narrowing is not expressible, since Rust cannot test a
        // `dyn IndexInput` for another trait (the module's declared run-time
        // narrowing divergence), so this takes Lucene's fallback branch; both
        // decoders produce the same integers. A caller that already holds a
        // narrowed input builds the segment decoder directly with
        // `MemorySegmentPostingDecodingUtil::new`.
        Ok(PostingDecodingUtil::new(input))
    }

    fn get_doc_values_range_support(&self) -> &'static dyn DocValuesRangeSupport {
        static PANAMA: PanamaDocValuesRangeSupport = PanamaDocValuesRangeSupport::INSTANCE;
        &PANAMA
    }

    fn get_doc_values_bulk_decode_support(&self) -> &'static dyn DocValuesBulkDecodeSupport {
        static PANAMA: PanamaDocValuesBulkDecodeSupport =
            PanamaDocValuesBulkDecodeSupport::INSTANCE;
        &PANAMA
    }
}
