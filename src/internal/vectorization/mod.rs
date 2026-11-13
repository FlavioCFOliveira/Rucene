//! Internal implementations to support SIMD vectorization.
//!
//! Port of `org.apache.lucene.internal.vectorization`. **This package is for
//! internal Lucene use only!**
//!
//! [`VectorizationProvider`] is the entry point: it is looked up once per
//! process by [`get_instance`] and hands out the backends that
//! [`crate::util::vector_util`], the flat-vector scorers, the postings reader
//! and the doc-values producer delegate their inner loops to. Lucene ships two
//! providers — a scalar one and one built on the Panama Vector API — and both
//! are ported here.
//!
//! In Lucene the package is split across two source trees: `src/java` holds the
//! provider indirection and the scalar backends, `src/java21` holds the Panama
//! implementations and the memory-segment scorers.
//!
//! The memory-segment scorers, the memory-segment postings and bulk vector
//! operations, and [`PanamaVectorizationProvider`], which registers them, read
//! a [`MemorySegment`](crate::store::MemorySegment) and exist only with the
//! `mmap` feature, which provides it. Without it the crate has no Panama
//! provider, and [`lookup`] returns [`DefaultVectorizationProvider`] — the
//! fallback Lucene's `VectorizationProvider.lookup` takes when the Panama
//! implementation is unavailable.
//!
//! | Rucene | Apache Lucene Core 10.5.0 (`src/java`) |
//! | --- | --- |
//! | [`VectorizationProvider`], [`get_instance`], [`ensure_caller`], [`lookup`], [`lookup_vector_module`], [`get_upper_java_feature_version`], [`VALID_CALLERS`], [`VALID_CALLER_FILES`] | `VectorizationProvider` |
//! | [`DefaultVectorizationProvider`] | `DefaultVectorizationProvider` |
//! | [`VectorUtilSupport`] | `VectorUtilSupport` |
//! | [`DefaultVectorUtilSupport`] | `DefaultVectorUtilSupport` |
//! | [`PostingDecodingUtil`] | `PostingDecodingUtil` |
//! | [`DocValuesRangeSupport`] | `DocValuesRangeSupport` |
//! | [`DefaultDocValuesRangeSupport`] | `DefaultDocValuesRangeSupport` |
//! | [`DocValuesBulkDecodeSupport`] | `DocValuesBulkDecodeSupport` |
//! | [`DefaultDocValuesBulkDecodeSupport`] | `DefaultDocValuesBulkDecodeSupport` |
//!
//! | Rucene | Apache Lucene Core 10.5.0 (`src/java21`) |
//! | --- | --- |
//! | [`PanamaVectorizationProvider`] | `PanamaVectorizationProvider` |
//! | [`PanamaVectorConstants`] | `PanamaVectorConstants` |
//! | [`PanamaVectorUtilSupport`] | `PanamaVectorUtilSupport` |
//! | [`PanamaDocValuesRangeSupport`] | `PanamaDocValuesRangeSupport` |
//! | [`PanamaDocValuesBulkDecodeSupport`] | `PanamaDocValuesBulkDecodeSupport` |
//! | [`MemorySegmentPostingDecodingUtil`] | `MemorySegmentPostingDecodingUtil` |
//! | [`MemorySegmentBulkVectorOps`] | `MemorySegmentBulkVectorOps` |
//! | [`Lucene99MemorySegmentFlatVectorsScorer`] | `Lucene99MemorySegmentFlatVectorsScorer` |
//! | [`Lucene99MemorySegmentByteVectorScorer`] | `Lucene99MemorySegmentByteVectorScorer` |
//! | [`Lucene99MemorySegmentByteVectorScorerSupplier`] | `Lucene99MemorySegmentByteVectorScorerSupplier` |
//! | [`Lucene99MemorySegmentFloatVectorScorer`] | `Lucene99MemorySegmentFloatVectorScorer` |
//! | [`Lucene99MemorySegmentFloatVectorScorerSupplier`] | `Lucene99MemorySegmentFloatVectorScorerSupplier` |
//! | [`Lucene99MemorySegmentScalarQuantizedVectorScorer`] | `Lucene99MemorySegmentScalarQuantizedVectorScorer` |
//!
//! # The Panama kernels run on lane arrays
//!
//! The Panama half of the package rests on two Java 21 features. One of them,
//! `java.lang.foreign.MemorySegment`, is ported onto `memmap2` in
//! [`crate::store::memory_segment`], so the memory-segment scorers here read
//! vectors in place from the mapped file.
//!
//! The other, the `jdk.incubator.vector` Vector API, is reproduced with
//! fixed-width lane arrays in safe, stable Rust: a species is its lane count,
//! taken from [`PanamaVectorConstants::preferred_vector_bitsize`] (the
//! `VectorShape.preferredShape()` HotSpot would pick on this CPU), and each
//! lane computes what the Java lane computes. The kernels keep Java's gates
//! (`Constants.HAS_FAST_VECTOR_FMA`, `HAS_FAST_SCALAR_FMA`,
//! `HAS_FAST_COMPRESS_MASK_CAST`, `PanamaVectorConstants.HAS_FAST_INTEGER_VECTORS`
//! and the vector-size branches), loop bounds, unrolling and tails, and the
//! float reductions use the order HotSpot's C2 compiler uses (see
//! [`PanamaVectorUtilSupport`]), so their results are Java's bit for bit
//! under the same gate values. The `tests.vectorsize` and
//! `tests.forceintegervectors` test properties are not read (declared on
//! [`PanamaVectorConstants`]).
//!
//! # Divergence from Lucene 10.5.0: the run-time narrowing is explicit
//!
//! Lucene decides at run time whether a given input or values object supports
//! the memory-segment path, with `instanceof` tests against
//! `MemorySegmentAccessInput`, `HasIndexSlice` and
//! `LegacyQuantizedByteVectorValues`, falling back to a delegate when a test
//! fails. Rust cannot test a trait object for a *different* trait, so those
//! tests are not expressible. Every affected entry point therefore comes in two
//! forms: the trait method, which takes Lucene's fallback branch, and an
//! inherent method taking the already-narrowed type, which reaches the
//! memory-segment implementation. The individual items name which is which.
//!
//! # Which provider [`lookup`] returns
//!
//! [`lookup`] runs Lucene's gates in Lucene's order. Its first gate is
//! `Constants.IS_HOTSPOT_VM`, and
//! [`crate::util::jvm::HotspotVMOptions::is_hotspot_vm`] reports `false` in a
//! Rust binary (a declared divergence of that type), so `lookup` returns
//! [`DefaultVectorizationProvider`], as Lucene does on a non-HotSpot VM. The
//! Panama provider is complete and can be constructed directly with
//! [`PanamaVectorizationProvider::new`], which applies Lucene's own
//! constructor guard.

#![deny(unsafe_code)]

pub mod default_doc_values_bulk_decode_support;
pub mod default_doc_values_range_support;
pub mod default_vector_util_support;
pub mod default_vectorization_provider;
pub mod doc_values_bulk_decode_support;
pub mod doc_values_range_support;
#[cfg(feature = "mmap")]
pub mod lucene99_memory_segment_byte_vector_scorer;
#[cfg(feature = "mmap")]
pub mod lucene99_memory_segment_byte_vector_scorer_supplier;
#[cfg(feature = "mmap")]
pub mod lucene99_memory_segment_flat_vectors_scorer;
#[cfg(feature = "mmap")]
pub mod lucene99_memory_segment_float_vector_scorer;
#[cfg(feature = "mmap")]
pub mod lucene99_memory_segment_float_vector_scorer_supplier;
#[cfg(feature = "mmap")]
pub mod lucene99_memory_segment_scalar_quantized_vector_scorer;
#[cfg(feature = "mmap")]
pub mod memory_segment_bulk_vector_ops;
#[cfg(feature = "mmap")]
pub mod memory_segment_posting_decoding_util;
pub mod panama_doc_values_bulk_decode_support;
pub mod panama_doc_values_range_support;
pub mod panama_vector_constants;
pub mod panama_vector_util_support;
#[cfg(feature = "mmap")]
pub mod panama_vectorization_provider;
pub mod posting_decoding_util;
pub mod vector_util_support;
pub mod vectorization_provider;

pub use default_doc_values_bulk_decode_support::DefaultDocValuesBulkDecodeSupport;
pub use default_doc_values_range_support::DefaultDocValuesRangeSupport;
pub use default_vector_util_support::DefaultVectorUtilSupport;
pub use default_vectorization_provider::DefaultVectorizationProvider;
pub use doc_values_bulk_decode_support::DocValuesBulkDecodeSupport;
pub use doc_values_range_support::DocValuesRangeSupport;
#[cfg(feature = "mmap")]
pub use lucene99_memory_segment_byte_vector_scorer::Lucene99MemorySegmentByteVectorScorer;
#[cfg(feature = "mmap")]
pub use lucene99_memory_segment_byte_vector_scorer_supplier::Lucene99MemorySegmentByteVectorScorerSupplier;
#[cfg(feature = "mmap")]
pub use lucene99_memory_segment_flat_vectors_scorer::Lucene99MemorySegmentFlatVectorsScorer;
#[cfg(feature = "mmap")]
pub use lucene99_memory_segment_float_vector_scorer::Lucene99MemorySegmentFloatVectorScorer;
#[cfg(feature = "mmap")]
pub use lucene99_memory_segment_float_vector_scorer_supplier::Lucene99MemorySegmentFloatVectorScorerSupplier;
#[cfg(feature = "mmap")]
pub use lucene99_memory_segment_scalar_quantized_vector_scorer::Lucene99MemorySegmentScalarQuantizedVectorScorer;
#[cfg(feature = "mmap")]
pub use memory_segment_bulk_vector_ops::MemorySegmentBulkVectorOps;
#[cfg(feature = "mmap")]
pub use memory_segment_posting_decoding_util::MemorySegmentPostingDecodingUtil;
pub use panama_doc_values_bulk_decode_support::PanamaDocValuesBulkDecodeSupport;
pub use panama_doc_values_range_support::PanamaDocValuesRangeSupport;
pub use panama_vector_constants::PanamaVectorConstants;
pub use panama_vector_util_support::PanamaVectorUtilSupport;
#[cfg(feature = "mmap")]
pub use panama_vectorization_provider::PanamaVectorizationProvider;
pub use posting_decoding_util::PostingDecodingUtil;
pub use vector_util_support::VectorUtilSupport;
pub use vectorization_provider::{
    ensure_caller, get_instance, get_upper_java_feature_version, lookup, lookup_vector_module,
    VectorizationProvider, DEFAULT_UPPER_JAVA_FEATURE_VERSION, UPPER_JAVA_FEATURE_VERSION_SYSPROP,
    VALID_CALLERS, VALID_CALLER_FILES,
};
