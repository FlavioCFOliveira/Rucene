//! Port of `org.apache.lucene.internal.vectorization.VectorizationProvider`.

#![deny(unsafe_code)]

use std::fmt::Debug;
use std::sync::{Arc, LazyLock};

use crate::codecs::hnsw::FlatVectorsScorer;
use crate::error::Result;
use crate::internal::vectorization::{
    DefaultDocValuesRangeSupport, DefaultVectorizationProvider, DocValuesBulkDecodeSupport,
    DocValuesRangeSupport, PostingDecodingUtil, VectorUtilSupport,
};
use crate::store::IndexInput;
use crate::util::jvm::HotspotVMOptions;

/// A provider of vectorization implementations.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.VectorizationProvider`.
///
/// Lucene picks an implementation at run time: an optimized one built on the
/// `jdk.incubator.vector` (Panama) API when the JVM makes it available, and the
/// scalar [`DefaultVectorizationProvider`] otherwise. Use [`get_instance`] to
/// obtain the singleton for the current runtime; see the [module docs](super)
/// for which implementations exist in this port.
///
/// # Divergences from Lucene 10.5.0
///
/// * **Trait, not an abstract class.** Java hides the constructor so that only
///   classes in the same package can extend `VectorizationProvider`; Rust has
///   no package visibility, so any crate could implement this trait. The
///   restriction is documentary here.
/// * **Free functions for the statics.** Java exposes
///   `VectorizationProvider.getInstance()` and the package-private
///   `lookup(boolean)` as static methods on the class. A `dyn`-compatible Rust
///   trait cannot carry them, so they are the module-level [`get_instance`] and
///   [`lookup`] functions.
/// * **`Send + Sync + Debug` bounds.** Not in the Java class; Rust needs them
///   because the singleton lives in a `static`.
pub trait VectorizationProvider: Send + Sync + Debug {
    /// Returns a singleton (stateless) [`VectorUtilSupport`] to support SIMD
    /// usage in [`crate::util::vector_util`].
    ///
    /// Equivalent to `VectorizationProvider.getVectorUtilSupport()`.
    fn get_vector_util_support(&self) -> &dyn VectorUtilSupport;

    /// Returns a [`FlatVectorsScorer`] that supports the Lucene99 format.
    ///
    /// Equivalent to `VectorizationProvider.getLucene99FlatVectorsScorer()`.
    fn get_lucene99_flat_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer>;

    /// Returns a [`FlatVectorsScorer`] that supports the Lucene99 scalar
    /// quantized format.
    ///
    /// Equivalent to
    /// `VectorizationProvider.getLucene99ScalarQuantizedVectorsScorer()`.
    fn get_lucene99_scalar_quantized_vectors_scorer(&self) -> Arc<dyn FlatVectorsScorer>;

    /// Creates a new [`PostingDecodingUtil`] for the given input.
    ///
    /// Equivalent to `VectorizationProvider.newPostingDecodingUtil(IndexInput)`.
    ///
    /// # Errors
    ///
    /// Returns any error raised while inspecting the input. Lucene declares
    /// `throws IOException` for the same reason: a provider may need to map the
    /// input before it can decide which decoder to hand back.
    fn new_posting_decoding_util(&self, input: Box<dyn IndexInput>) -> Result<PostingDecodingUtil>;

    /// Returns a [`DocValuesRangeSupport`] for SIMD-accelerated range
    /// evaluation.
    ///
    /// Equivalent to `VectorizationProvider.getDocValuesRangeSupport()`,
    /// including the default implementation that returns the scalar singleton.
    fn get_doc_values_range_support(&self) -> &'static dyn DocValuesRangeSupport {
        static DEFAULT: DefaultDocValuesRangeSupport = DefaultDocValuesRangeSupport::INSTANCE;
        &DEFAULT
    }

    /// Returns a [`DocValuesBulkDecodeSupport`] instance for bulk numeric value
    /// decode.
    ///
    /// Equivalent to `VectorizationProvider.getDocValuesBulkDecodeSupport()`.
    fn get_doc_values_bulk_decode_support(&self) -> &'static dyn DocValuesBulkDecodeSupport;
}

/// The classes Lucene allows to call `VectorizationProvider.getInstance()`.
///
/// Equivalent to the private `VectorizationProvider.VALID_CALLERS` set. Lucene
/// enforces it in `ensureCaller()` with a `StackWalker`, throwing
/// `IllegalCallerException` for anybody else.
///
/// # Divergence from Lucene 10.5.0
///
/// Rust has no stack walker that can report the *caller's* type, so
/// [`get_instance`] cannot enforce this and the list is kept as documentation
/// of who is meant to use the provider. The equivalent Rucene call sites are
/// `crate::codecs::hnsw::FlatVectorScorerUtil`, `crate::util::vector_util`, the
/// Lucene104 postings reader and the Lucene90 doc-values producer.
pub const VALID_CALLERS: &[&str] = &[
    "org.apache.lucene.codecs.hnsw.FlatVectorScorerUtil",
    "org.apache.lucene.util.VectorUtil",
    "org.apache.lucene.codecs.lucene104.Lucene104PostingsReader",
    "org.apache.lucene.codecs.lucene104.PostingIndexInput",
    "org.apache.lucene.codecs.lucene90.Lucene90DocValuesProducer",
    "org.apache.lucene.tests.util.TestSysoutsLimits",
];

/// The warning Lucene logs when the runtime is not a HotSpot VM.
const NOT_HOTSPOT_VM_WARNING: &str =
    "Java runtime is not using Hotspot VM; Java vector incubator API can't be enabled.";

/// Holds the singleton, initialized on first use.
///
/// Equivalent to the private `VectorizationProvider.Holder` class, which exists
/// in Java to prevent a classloading deadlock. [`LazyLock`] gives the same
/// initialize-once-on-first-access behaviour.
static HOLDER: LazyLock<Arc<dyn VectorizationProvider>> = LazyLock::new(|| lookup(false));

/// Returns the default instance of the provider matching the vectorization
/// possibilities of the actual runtime.
///
/// Equivalent to `VectorizationProvider.getInstance()`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java throws `IllegalCallerException` when the caller is not one of
/// [`VALID_CALLERS`]. Rust cannot identify the calling type at run time, so
/// this function performs no caller check.
pub fn get_instance() -> Arc<dyn VectorizationProvider> {
    Arc::clone(&HOLDER)
}

/// Selects the provider that matches the current runtime.
///
/// Equivalent to the package-private `VectorizationProvider.lookup(boolean)`,
/// which Lucene exposes for its own tests.
///
/// Lucene's first gate is `Constants.IS_HOTSPOT_VM`: the Java vector incubator
/// module only works on HotSpot, and every other case falls back to the scalar
/// provider with a logged warning. `HotspotVMOptions::is_hotspot_vm()`
/// reproduces Lucene's own non-HotSpot branch and is permanently `false` in a
/// Rust binary, so this port always takes that branch and returns
/// [`DefaultVectorizationProvider`]. The remaining gates Lucene evaluates
/// afterwards — the JVMCI check, the readability of `jdk.incubator.vector`, the
/// `tests.vectorsize` / `tests.forceintegervectors` system properties, and the
/// client-VM check — are unreachable from here and are described in the
/// [module docs](super).
///
/// `test_mode` is therefore unused: in Lucene it only relaxes two of those
/// unreachable gates.
pub fn lookup(test_mode: bool) -> Arc<dyn VectorizationProvider> {
    let _ = test_mode;
    if !HotspotVMOptions::is_hotspot_vm() {
        log::warn!("{NOT_HOTSPOT_VM_WARNING}");
    }
    Arc::new(DefaultVectorizationProvider::new())
}
