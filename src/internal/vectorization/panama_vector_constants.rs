//! Port of `org.apache.lucene.internal.vectorization.PanamaVectorConstants`.

#![deny(unsafe_code)]

use crate::util::Constants;

/// Shared constants for implementations that take advantage of the Panama
/// Vector API.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaVectorConstants`.
///
/// # Divergence from Lucene 10.5.0: there are no vector lanes
///
/// Java derives every constant here from `jdk.incubator.vector`:
/// `PREFERRED_VECTOR_BITSIZE` is `VectorShape.preferredShape().vectorBitSize()`
/// (overridable with the `tests.vectorsize` system property), and the three
/// `VectorSpecies` fields are the `long`, `int` and `double` species of that
/// shape. Stable Rust has no portable SIMD — `std::simd` is nightly-only and
/// this crate's MSRV is 1.80 — so there is no shape to ask for and no species
/// to name.
///
/// This port therefore reports a preferred bit size of **zero**, meaning "no
/// vector lanes". That is not an arbitrary sentinel: every kernel in the Panama
/// classes guards its vector path on a minimum width (`>= 128`, `== 256`,
/// `>= 512`, `vectorByteSize() >= 32`), so a width of zero makes each of those
/// guards select the scalar branch that Lucene itself falls back to. The
/// selection of the scalar kernels in this port is thus Lucene's own decision
/// evaluated on this platform's real capability, not a Rucene-specific rule.
///
/// The `tests.vectorsize` and `tests.forceintegervectors` system properties are
/// not read, because they exist only to widen or narrow a vector path that does
/// not exist here. The species fields are not ported for the same reason.
#[derive(Debug, Clone, Copy)]
pub struct PanamaVectorConstants;

impl PanamaVectorConstants {
    /// Preferred width in bits for vectors.
    ///
    /// Equivalent to `PanamaVectorConstants.PREFERRED_VECTOR_BITSIZE`. Always
    /// zero here; see the type documentation.
    pub const PREFERRED_VECTOR_BITSIZE: i32 = 0;

    /// Preferred width in bytes for vectors.
    ///
    /// Equivalent to `VectorSpecies.vectorByteSize()` on any of the species
    /// Lucene derives from [`PREFERRED_VECTOR_BITSIZE`](Self::PREFERRED_VECTOR_BITSIZE).
    pub const PREFERRED_VECTOR_BYTESIZE: i32 = Self::PREFERRED_VECTOR_BITSIZE / 8;

    /// Whether integer vectors can be trusted to actually be fast.
    ///
    /// Equivalent to `PanamaVectorConstants.HAS_FAST_INTEGER_VECTORS`.
    ///
    /// Lucene computes this as `TESTS_FORCE_INTEGER_VECTORS || !(OS_ARCH ==
    /// "amd64" && PREFERRED_VECTOR_BITSIZE < 256)`, working around HotSpot
    /// missing some SSE intrinsics. That formula cannot be applied here: it
    /// asks whether the *available* integer vectors are fast, and there are
    /// none at all, so the answer is unconditionally `false`.
    pub const HAS_FAST_INTEGER_VECTORS: bool = false;

    /// Returns whether Lucene's own architecture work-around would disqualify
    /// integer vectors on this machine.
    ///
    /// This is the `OS_ARCH.equals("amd64") && PREFERRED_VECTOR_BITSIZE < 256`
    /// half of Lucene's formula, evaluated against
    /// [`Constants::os_arch`]. It is exposed so the reasoning behind
    /// [`HAS_FAST_INTEGER_VECTORS`](Self::HAS_FAST_INTEGER_VECTORS) stays
    /// checkable; it does not, by itself, enable any vector path.
    pub fn is_amd64_without_avx2() -> bool {
        // Rust names the architecture `x86_64` where the JVM names it `amd64`.
        Constants::os_arch() == "x86_64" && Self::PREFERRED_VECTOR_BITSIZE < 256
    }
}
