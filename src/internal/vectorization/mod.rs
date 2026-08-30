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
//! | Rucene | Apache Lucene Core 10.5.0 (`src/java`) |
//! | --- | --- |
//! | [`VectorizationProvider`], [`get_instance`], [`lookup`], [`VALID_CALLERS`] | `VectorizationProvider` |
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
//! # Divergence from Lucene 10.5.0: the SIMD kernels are scalar
//!
//! The Panama half of the package rests on two Java 21 features. One of them,
//! `java.lang.foreign.MemorySegment`, is already ported — the crate maps it onto
//! `memmap2` in [`crate::store::memory_segment`] — so the memory-segment
//! scorers here really do read vectors in place from the mapped file instead of
//! copying them to the heap. That is the structural reason those classes exist,
//! and it survives the port intact.
//!
//! The other, the `jdk.incubator.vector` Vector API, does not: Rust's portable
//! SIMD (`std::simd`) is nightly-only and this crate targets stable Rust with
//! an MSRV of 1.80. Every Panama kernel in Lucene is written twice — a lane
//! path, and a scalar loop that finishes the elements the lanes do not cover —
//! and each lane path is guarded on a minimum vector width. This port sets
//! [`PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE`] to zero, which makes
//! every one of those guards fail, so **the computation is Lucene's own scalar
//! remainder**, taken for every element rather than only for the tail. Each
//! affected item states this at its point of divergence.
//!
//! Two consequences follow:
//!
//! * **Float results can differ in their low bits** from a JVM running with
//!   `--add-modules jdk.incubator.vector`, because a SIMD reduction adds the
//!   partial sums in a different order and floating-point addition is not
//!   associative. That split exists inside Lucene too, which is why the crate
//!   pins its reference float results to the scalar path; see
//!   [`crate::util::vector_util`]. Integer results — byte dot products, square
//!   distances, `int4BitDotProduct`, `findNextGEQ` — are exact and identical on
//!   every path.
//! * **Lucene's SIMD-only tuning knobs are not ported**: the `tests.vectorsize`
//!   and `tests.forceintegervectors` system properties, the
//!   `org.apache.lucene.vectorization.upperJavaFeatureVersion` property and the
//!   `VectorSpecies` fields select between lane widths that do not exist here.
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
//! [`lookup`] is a faithful port and returns [`DefaultVectorizationProvider`],
//! because Lucene's first gate is `Constants.IS_HOTSPOT_VM` and
//! [`crate::util::jvm::HotspotVMOptions`] — itself a faithful port of Lucene's
//! non-HotSpot fallback — reports `false` in a Rust binary. The Panama provider
//! is thus unreachable through `lookup` by Lucene's own logic rather than by a
//! Rucene-specific rule; it is a complete implementation and can be constructed
//! directly, and it becomes reachable the day that gate changes.

#![deny(unsafe_code)]

pub mod default_doc_values_bulk_decode_support;
pub mod default_doc_values_range_support;
pub mod default_vector_util_support;
pub mod default_vectorization_provider;
pub mod doc_values_bulk_decode_support;
pub mod doc_values_range_support;
pub mod lucene99_memory_segment_byte_vector_scorer;
pub mod lucene99_memory_segment_byte_vector_scorer_supplier;
pub mod lucene99_memory_segment_flat_vectors_scorer;
pub mod lucene99_memory_segment_float_vector_scorer;
pub mod lucene99_memory_segment_float_vector_scorer_supplier;
pub mod lucene99_memory_segment_scalar_quantized_vector_scorer;
pub mod memory_segment_bulk_vector_ops;
pub mod memory_segment_posting_decoding_util;
pub mod panama_doc_values_bulk_decode_support;
pub mod panama_doc_values_range_support;
pub mod panama_vector_constants;
pub mod panama_vector_util_support;
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
pub use lucene99_memory_segment_byte_vector_scorer::Lucene99MemorySegmentByteVectorScorer;
pub use lucene99_memory_segment_byte_vector_scorer_supplier::Lucene99MemorySegmentByteVectorScorerSupplier;
pub use lucene99_memory_segment_flat_vectors_scorer::Lucene99MemorySegmentFlatVectorsScorer;
pub use lucene99_memory_segment_float_vector_scorer::Lucene99MemorySegmentFloatVectorScorer;
pub use lucene99_memory_segment_float_vector_scorer_supplier::Lucene99MemorySegmentFloatVectorScorerSupplier;
pub use lucene99_memory_segment_scalar_quantized_vector_scorer::Lucene99MemorySegmentScalarQuantizedVectorScorer;
pub use memory_segment_bulk_vector_ops::MemorySegmentBulkVectorOps;
pub use memory_segment_posting_decoding_util::MemorySegmentPostingDecodingUtil;
pub use panama_doc_values_bulk_decode_support::PanamaDocValuesBulkDecodeSupport;
pub use panama_doc_values_range_support::PanamaDocValuesRangeSupport;
pub use panama_vector_constants::PanamaVectorConstants;
pub use panama_vector_util_support::PanamaVectorUtilSupport;
pub use panama_vectorization_provider::PanamaVectorizationProvider;
pub use posting_decoding_util::PostingDecodingUtil;
pub use vector_util_support::VectorUtilSupport;
pub use vectorization_provider::{get_instance, lookup, VectorizationProvider, VALID_CALLERS};
