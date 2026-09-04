//! Port of `org.apache.lucene.internal.vectorization.PanamaVectorizationProvider`.

#![deny(unsafe_code)]

use std::sync::Arc;

use crate::codecs::hnsw::FlatVectorsScorer;
use crate::error::Result;
use crate::internal::vectorization::{
    DocValuesBulkDecodeSupport, DocValuesRangeSupport, Lucene99MemorySegmentFlatVectorsScorer,
    Lucene99MemorySegmentScalarQuantizedVectorScorer, PanamaDocValuesBulkDecodeSupport,
    PanamaDocValuesRangeSupport, PanamaVectorConstants, PanamaVectorUtilSupport,
    PostingDecodingUtil, VectorUtilSupport, VectorizationProvider,
};
use crate::store::IndexInput;

/// Compile-time record that Lucene's memory-segment postings fast path is
/// unreachable in this port, so that the day
/// [`PanamaVectorConstants::HAS_FAST_INTEGER_VECTORS`] becomes true,
/// [`PanamaVectorizationProvider::new_posting_decoding_util`] is revisited.
const _: () = assert!(!PanamaVectorConstants::HAS_FAST_INTEGER_VECTORS);

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
/// [`lookup`](super::lookup) never returns this provider. Lucene's first gate
/// is `Constants.IS_HOTSPOT_VM`, and
/// [`HotspotVMOptions::is_hotspot_vm`](crate::util::jvm::HotspotVMOptions::is_hotspot_vm)
/// reproduces Lucene's own non-HotSpot fallback and reports `false` in a Rust
/// binary, so the scalar provider is selected by Lucene's own logic. This type
/// is nevertheless a complete implementation and can be constructed directly;
/// it becomes reachable through `lookup` the day that gate changes.
///
/// # Divergences from Lucene 10.5.0
///
/// * **The constructor cannot fail.** Java refuses to build the provider when
///   `PanamaVectorConstants.PREFERRED_VECTOR_BITSIZE < 128`, throwing
///   `UnsupportedOperationException` for `lookup` to catch, because its
///   kernels would then be slower than the scalar ones. This port's kernels
///   *are* the scalar ones (see [`PanamaVectorUtilSupport`]), so there is
///   nothing to refuse and the guard would reject a provider that is correct at
///   any width; it is therefore not reproduced.
/// * **No JDK-8309727 warm-up.** Java loads one `FloatVector` inside
///   `AccessController.doPrivileged` to work around a JDK initialization bug.
///   There is no Vector API and no security manager here.
/// * **A different log line.** Java logs "Java vector incubator API enabled;
///   uses preferredBitSize=…". Saying that here would be false, so the port
///   logs what actually happened.
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
    /// constructor, minus the two guards described on the type.
    pub fn new() -> Self {
        log::info!(
            "Panama vectorization provider constructed with scalar kernels; \
             preferredBitSize={} (stable Rust has no portable SIMD)",
            PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE
        );
        Self {
            vector_util_support: PanamaVectorUtilSupport::new(),
        }
    }
}

impl Default for PanamaVectorizationProvider {
    fn default() -> Self {
        Self::new()
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
        // Lucene additionally tests `input instanceof MemorySegmentAccessInput`
        // and, when the whole file fits in one segment, returns a
        // `MemorySegmentPostingDecodingUtil`. That branch is guarded on
        // `HAS_FAST_INTEGER_VECTORS`, which is unconditionally false here, so
        // Lucene's own code returns the base decoder; the narrowing would in
        // any case not be expressible, since Rust cannot test a
        // `dyn IndexInput` for another trait. A caller that already holds a
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
