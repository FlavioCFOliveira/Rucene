//! Port of `org.apache.lucene.internal.vectorization.PanamaVectorConstants`.

#![deny(unsafe_code)]

use std::sync::LazyLock;

use crate::util::jvm::HotspotVMOptions;
use crate::util::Constants;

/// Shared constants for implementations that take advantage of the Panama
/// Vector API.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaVectorConstants`.
///
/// Java derives every constant from `jdk.incubator.vector`:
/// `PREFERRED_VECTOR_BITSIZE` is `VectorShape.preferredShape().vectorBitSize()`
/// and the three species fields are the `long`, `int` and `double` species of
/// that shape. The port computes the same preferred shape from the HotSpot CPU
/// options that decide it (see [`preferred_shape_bit_size`]), and represents a
/// species by its lane count: the Panama kernels of this package run on
/// fixed-width lane arrays of exactly that many lanes, so each lane computes
/// what the corresponding Java lane computes.
///
/// # Divergence from Lucene 10.5.0, declared
///
/// The `tests.vectorsize` and `tests.forceintegervectors` system properties
/// (`VectorizationProvider.TESTS_VECTOR_SIZE` and
/// `TESTS_FORCE_INTEGER_VECTORS`) are not read: they exist only for Lucene's
/// own tests, and Lucene refuses the Panama provider outside test mode when
/// either is set. Both therefore keep their unset values here: the preferred
/// size is the platform's, and integer vectors are not forced.
///
/// [`preferred_shape_bit_size`]: PanamaVectorConstants::preferred_shape_bit_size
#[derive(Debug, Clone, Copy)]
pub struct PanamaVectorConstants;

/// `PanamaVectorConstants.PREFERRED_VECTOR_BITSIZE`, computed once, as Java's
/// `static final` field is computed once when the class initialises.
static PREFERRED_VECTOR_BITSIZE: LazyLock<i32> =
    LazyLock::new(PanamaVectorConstants::preferred_shape_bit_size);

impl PanamaVectorConstants {
    /// Preferred width in bits for vectors.
    ///
    /// Equivalent to `PanamaVectorConstants.PREFERRED_VECTOR_BITSIZE`:
    /// `VectorShape.preferredShape().vectorBitSize()`, see
    /// [`preferred_shape_bit_size`](Self::preferred_shape_bit_size).
    pub fn preferred_vector_bitsize() -> i32 {
        *PREFERRED_VECTOR_BITSIZE
    }

    /// Preferred width in bytes for vectors.
    ///
    /// Equivalent to `VectorSpecies.vectorByteSize()` on any species of the
    /// preferred shape (`ByteVector.SPECIES_PREFERRED.vectorByteSize()`).
    pub fn preferred_vector_bytesize() -> i32 {
        Self::preferred_vector_bitsize() / 8
    }

    /// Whether integer vectors can be trusted to actually be fast.
    ///
    /// Equivalent to `PanamaVectorConstants.HAS_FAST_INTEGER_VECTORS`:
    /// `TESTS_FORCE_INTEGER_VECTORS || !(OS_ARCH == "amd64" &&
    /// PREFERRED_VECTOR_BITSIZE < 256)`, with `TESTS_FORCE_INTEGER_VECTORS`
    /// false (see the type documentation).
    pub fn has_fast_integer_vectors() -> bool {
        !Self::is_amd64_without_avx2()
    }

    /// `OS_ARCH.equals("amd64") && PREFERRED_VECTOR_BITSIZE < 256`, the
    /// HotSpot work-around half of
    /// [`has_fast_integer_vectors`](Self::has_fast_integer_vectors).
    pub fn is_amd64_without_avx2() -> bool {
        Constants::os_arch() == "amd64" && Self::preferred_vector_bitsize() < 256
    }

    /// Lane count of `PRERERRED_INT_SPECIES` (and of `FLOAT_SPECIES`, which
    /// Lucene derives from it with `withLanes(float.class)`).
    pub fn int_lanes() -> usize {
        (Self::preferred_vector_bitsize() / 32) as usize
    }

    /// Lane count of `PRERERRED_LONG_SPECIES` and `LongVector.SPECIES_PREFERRED`.
    pub fn long_lanes() -> usize {
        (Self::preferred_vector_bitsize() / 64) as usize
    }

    /// `VectorShape.preferredShape().vectorBitSize()` on the running CPU.
    ///
    /// The JDK computes it as `max(VectorSupport.getMaxLaneCount(byte.class)
    /// * 8, 64)` (`VectorShape.getMaxVectorBitSize`), and HotSpot's lane count
    /// is `Matcher::max_vector_size(T_BYTE)`, the byte vector width C2 can
    /// use. The port computes that width from the HotSpot options
    /// [`HotspotVMOptions::get`] answers for this CPU, by the rule of
    /// OpenJDK 25 `Matcher::vector_width_in_bytes` (read as specification):
    ///
    /// * x86-64: 64 bytes with `UseAVX=3` and AVX-512BW, 32 bytes with
    ///   `UseAVX=3` without it or with `UseAVX=2`, 16 bytes otherwise; then
    ///   capped at `MaxVectorSize`;
    /// * aarch64: `MaxVectorSize`, which HotSpot sets to the SVE vector length
    ///   or to the 16-byte NEON width;
    /// * any other architecture, where no option is answered: no C2 vector
    ///   width, so the JDK's 64-bit floor.
    ///
    /// Measured on the JDK 25 oracle (x86-64, AVX2, no AVX-512): 256 by
    /// default, 128 with `-XX:UseAVX=1`, `-XX:UseAVX=0` or
    /// `-XX:MaxVectorSize=16`, 64 with `-XX:MaxVectorSize=8` or `4`. The
    /// `XX:<name>` environment variables are this port's form of those flags.
    pub fn preferred_shape_bit_size() -> i32 {
        let option = |name: &str| HotspotVMOptions::get(name).and_then(|v| v.parse::<i64>().ok());
        let byte_width: i64 = match Constants::os_arch().as_str() {
            "amd64" | "x86_64" => {
                let use_avx = option("UseAVX").unwrap_or(0);
                let size = if use_avx > 2 {
                    if Self::cpu_has_avx512bw() {
                        64
                    } else {
                        32
                    }
                } else if use_avx > 1 {
                    32
                } else {
                    16
                };
                option("MaxVectorSize").map_or(0, |max| size.min(max))
            }
            "aarch64" => option("MaxVectorSize").unwrap_or(0),
            _ => 0,
        };
        // `Matcher::max_vector_size` answers 0 below 4 bytes; the JDK floors
        // the shape at 64 bits either way.
        let byte_width = if byte_width < 4 { 0 } else { byte_width };
        (byte_width * 8).max(64) as i32
    }

    #[cfg(target_arch = "x86_64")]
    fn cpu_has_avx512bw() -> bool {
        std::arch::is_x86_feature_detected!("avx512bw")
    }

    #[cfg(not(target_arch = "x86_64"))]
    fn cpu_has_avx512bw() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_size_is_a_power_of_two_of_at_least_64_bits() {
        let bits = PanamaVectorConstants::preferred_vector_bitsize();
        assert!(bits >= 64 && (bits as u32).is_power_of_two(), "{bits}");
        assert_eq!(PanamaVectorConstants::preferred_vector_bytesize(), bits / 8);
        assert_eq!(PanamaVectorConstants::int_lanes() as i32, bits / 32);
    }

    #[test]
    fn integer_vectors_follow_lucene_rule() {
        let expected = !(Constants::os_arch() == "amd64"
            && PanamaVectorConstants::preferred_vector_bitsize() < 256);
        assert_eq!(PanamaVectorConstants::has_fast_integer_vectors(), expected);
    }
}
