//! Port of `org.apache.lucene.internal.vectorization.PanamaVectorUtilSupport`.

#![deny(unsafe_code)]

use std::sync::LazyLock;

use crate::error::{LuceneError, Result};
use crate::internal::vectorization::default_vector_util_support::{
    check_same_dimensions, check_single_packed, java_math_max, java_math_min, ScalarQuantizer,
};
use crate::internal::vectorization::{
    DefaultVectorUtilSupport, PanamaVectorConstants, VectorUtilSupport,
};
use crate::util::{vector_util, Constants};

/// `Constants.HAS_FAST_VECTOR_FMA`, evaluated once, on first use, as Java's
/// `static final` field is evaluated once when `Constants` initialises
/// (`Constants.java:109`).
static HAS_FAST_VECTOR_FMA: LazyLock<bool> = LazyLock::new(Constants::has_fast_vector_fma);

/// `Constants.HAS_FAST_COMPRESS_MASK_CAST`, evaluated once (`Constants.java:115`).
static HAS_FAST_COMPRESS_MASK_CAST: LazyLock<bool> =
    LazyLock::new(Constants::has_fast_compress_mask_cast);

/// Whether the vector form of [`fma_vec`] fuses: `Constants.HAS_FAST_VECTOR_FMA`.
pub fn use_vector_fma() -> bool {
    *HAS_FAST_VECTOR_FMA
}

// ---------------------------------------------------------------------------
// Lanes
// ---------------------------------------------------------------------------

/// A `FloatVector` of `N` lanes.
///
/// Each lane computes exactly what the corresponding lane of a
/// `jdk.incubator.vector.FloatVector` computes: lanewise IEEE single
/// operations, `Math.min`/`Math.max` semantics, one rounding for `fma`. The
/// lane count is the species length Lucene selects, so loop bounds, tails and
/// the per-lane accumulation are Java's.
#[derive(Debug, Clone, Copy)]
pub(crate) struct FloatVector<const N: usize>(pub(crate) [f32; N]);

impl<const N: usize> FloatVector<N> {
    /// `FloatVector.zero(species)`.
    #[inline]
    pub(crate) fn zero() -> Self {
        Self([0.0; N])
    }

    /// `FloatVector.broadcast(species, e)`.
    #[inline]
    pub(crate) fn broadcast(e: f32) -> Self {
        Self([e; N])
    }

    /// `FloatVector.fromArray(species, a, offset)`.
    #[inline]
    pub(crate) fn from_array(a: &[f32], offset: usize) -> Self {
        let mut v = [0.0; N];
        v.copy_from_slice(&a[offset..offset + N]);
        Self(v)
    }

    /// `FloatVector.fromMemorySegment(species, seg, offset, LITTLE_ENDIAN)`.
    #[inline]
    pub(crate) fn from_le_bytes(seg: &[u8], offset: usize) -> Self {
        let mut v = [0.0; N];
        for (k, lane) in v.iter_mut().enumerate() {
            let at = offset + 4 * k;
            *lane = f32::from_le_bytes([seg[at], seg[at + 1], seg[at + 2], seg[at + 3]]);
        }
        Self(v)
    }

    #[inline]
    fn map2(self, o: Self, f: impl Fn(f32, f32) -> f32) -> Self {
        Self(std::array::from_fn(|k| f(self.0[k], o.0[k])))
    }

    /// `FloatVector.add(v)`.
    #[inline]
    pub(crate) fn add(self, o: Self) -> Self {
        self.map2(o, |a, b| a + b)
    }

    /// `FloatVector.sub(v)`.
    #[inline]
    pub(crate) fn sub(self, o: Self) -> Self {
        self.map2(o, |a, b| a - b)
    }

    /// `FloatVector.mul(v)`.
    #[inline]
    pub(crate) fn mul(self, o: Self) -> Self {
        self.map2(o, |a, b| a * b)
    }

    /// `FloatVector.min(e)`: `Math.min(lane, e)`.
    #[inline]
    fn min_scalar(self, e: f32) -> Self {
        self.map2(Self::broadcast(e), java_math_min)
    }

    /// `FloatVector.max(e)`: `Math.max(lane, e)`.
    #[inline]
    fn max_scalar(self, e: f32) -> Self {
        self.map2(Self::broadcast(e), java_math_max)
    }

    /// `FloatVector.fma(b, c)`: `Math.fma` on every lane.
    #[inline]
    pub(crate) fn fma(self, b: Self, c: Self) -> Self {
        Self(std::array::from_fn(|k| self.0[k].mul_add(b.0[k], c.0[k])))
    }

    /// `FloatVector.reduceLanes(VectorOperators.ADD)` as C2 computes it.
    ///
    /// The Vector API leaves the order of a floating-point reduction
    /// unspecified. Measured on the JDK 25 oracle (x86-64, AVX2), HotSpot's C2
    /// intrinsic adds the upper half of the lanes to the lower half,
    /// repeatedly, without an identity term: for 8 lanes
    /// `((v0+v4)+(v2+v6)) + ((v1+v5)+(v3+v7))`, matching 2000 of 2000 random
    /// vectors, and the same halving for 4 lanes; a reduction of `-0.0` lanes
    /// is `-0.0`. The same JVM, before C2 compiles the caller (interpreter,
    /// C1), runs the Java fallback, which adds the lanes in order onto `0.0f`
    /// and gives a different low bit for 857 of those 2000 vectors. The
    /// compiled order is the one a running Lucene reaches and keeps; it is the
    /// one reproduced here.
    #[inline]
    pub(crate) fn reduce_add(self) -> f32 {
        let mut w = self.0;
        let mut n = N;
        while n > 1 {
            n /= 2;
            for k in 0..n {
                w[k] += w[k + n];
            }
        }
        w[0]
    }
}

/// `PanamaVectorUtilSupport.fma(FloatVector, FloatVector, FloatVector)`
/// (`PanamaVectorUtilSupport.java:91-97`): `a.fma(b, c)` when
/// `Constants.HAS_FAST_VECTOR_FMA`, else `a.mul(b).add(c)`.
#[inline]
pub(crate) fn fma_vec<const N: usize>(
    a: FloatVector<N>,
    b: FloatVector<N>,
    c: FloatVector<N>,
) -> FloatVector<N> {
    if use_vector_fma() {
        a.fma(b, c)
    } else {
        a.mul(b).add(c)
    }
}

/// `PanamaVectorUtilSupport.fma(float, float, float)`
/// (`PanamaVectorUtilSupport.java:99-106`): `Math.fma` when
/// `Constants.HAS_FAST_SCALAR_FMA`, else `a * b + c`.
#[inline]
pub(crate) fn fma(a: f32, b: f32, c: f32) -> f32 {
    if vector_util::use_fma() {
        a.mul_add(b, c)
    } else {
        a * b + c
    }
}

/// `VectorSpecies.loopBound(length)`: the largest multiple of `lanes` not
/// above `length` (`lanes` is a power of two).
#[inline]
pub(crate) fn loop_bound(length: usize, lanes: usize) -> usize {
    length - length % lanes
}

/// Calls `$f::<N>($args)` with `N` the float lane count `$lanes`.
macro_rules! with_float_lanes {
    ($lanes:expr, $f:ident ( $($arg:expr),* $(,)? )) => {
        match $lanes {
            2 => $f::<2>($($arg),*),
            4 => $f::<4>($($arg),*),
            8 => $f::<8>($($arg),*),
            16 => $f::<16>($($arg),*),
            32 => $f::<32>($($arg),*),
            64 => $f::<64>($($arg),*),
            n => unreachable!("no vector shape has {n} float lanes"),
        }
    };
}
pub(crate) use with_float_lanes;

/// `PanamaVectorUtilSupport.VECTOR_BITSIZE`.
#[inline]
fn vector_bitsize() -> i32 {
    PanamaVectorConstants::preferred_vector_bitsize()
}

/// Lane count of `FLOAT_SPECIES` (`INT_SPECIES.withLanes(float.class)`), of
/// `INT_SPECIES`, and — when `VECTOR_BITSIZE >= 256` — of `BYTE_SPECIES` and
/// `SHORT_SPECIES`, whose shapes are a quarter and a half of it.
#[inline]
fn float_lanes() -> usize {
    PanamaVectorConstants::int_lanes()
}

/// Byte species length and chunk of `PanamaVectorUtilSupport.Int4Constants`.
#[inline]
fn int4_constants() -> (usize, usize) {
    let bits = vector_bitsize();
    if bits >= 512 {
        (64, 8192)
    } else if bits == 256 {
        (32, 4096)
    } else {
        (16, 2048)
    }
}

fn check_same_dimensions_f32(a: &[f32], b: &[f32]) -> Result<()> {
    if a.len() != b.len() {
        return Err(LuceneError::illegal_argument(format!(
            "vector dimensions differ: {}!={}",
            a.len(),
            b.len()
        )));
    }
    Ok(())
}

// Lane conversions of the byte kernels.

/// `B2S` (`signed`) or `ZERO_EXTEND_B2S`.
#[inline]
fn b2s(x: u8, signed: bool) -> i16 {
    if signed {
        i16::from(x as i8)
    } else {
        i16::from(x)
    }
}

/// `S2I` (`signed`) or `ZERO_EXTEND_S2I`.
#[inline]
fn s2i(x: i16, signed: bool) -> i32 {
    if signed {
        i32::from(x)
    } else {
        i32::from(x as u16)
    }
}

/// `B2I` (`signed`) or `ZERO_EXTEND_B2I`.
#[inline]
fn b2i(x: u8, signed: bool) -> i32 {
    if signed {
        i32::from(x as i8)
    } else {
        i32::from(x)
    }
}

/// `ByteVector.mul` on one lane: the low byte of the product.
#[inline]
fn mul8(a: u8, b: u8) -> u8 {
    a.wrapping_mul(b)
}

/// `ByteVector.lanewise(LSHR, 4)` on one lane.
#[inline]
fn lshr4(a: u8) -> u8 {
    a >> 4
}

/// The short-lane accumulation of the int4 kernels.
///
/// Java reinterprets byte vectors as short vectors (lane `m` holding bytes
/// `2m` and `2m + 1`, little-endian) and adds into `ShortVector`
/// accumulators that wrap at 16 bits, restarting every `CHUNK` bytes; each
/// chunk is then folded by reinterpreting the accumulators as ints and adding
/// both 16-bit halves as unsigned values. `lane(e)` returns the two shorts
/// that lane `m` adds to `acc0` and `acc1` for the byte pair starting at the
/// absolute index `e`. Integer lane sums wrap and commute, so only the 16-bit
/// lanes need to be kept apart.
#[inline]
fn int4_lanes(limit: usize, lane: impl Fn(usize) -> (i16, i16)) -> i32 {
    let (bytes, chunk) = int4_constants();
    let shorts = bytes / 2;
    let mut sum = 0i32;
    let mut i = 0;
    while i < limit {
        let mut acc0 = [0i16; 32];
        let mut acc1 = [0i16; 32];
        let inner_limit = (limit - i).min(chunk);
        let mut j = 0;
        while j < inner_limit {
            for m in 0..shorts {
                let (x, y) = lane(i + j + 2 * m);
                acc0[m] = acc0[m].wrapping_add(x);
                acc1[m] = acc1[m].wrapping_add(y);
            }
            j += bytes;
        }
        let mut chunk_sum = 0i32;
        for m in 0..shorts {
            chunk_sum = chunk_sum
                .wrapping_add(i32::from(acc0[m] as u16))
                .wrapping_add(i32::from(acc1[m] as u16));
        }
        sum = sum.wrapping_add(chunk_sum);
        i += chunk;
    }
    sum
}

/// `VectorUtil` methods implemented with the Panama incubating vector API.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaVectorUtilSupport`.
///
/// Besides implementing [`VectorUtilSupport`], this type carries the `public
/// static` kernels that the `Lucene99MemorySegment*` scorers call to score
/// straight out of a memory-mapped segment.
///
/// Every kernel follows Java's branch structure: the gates
/// (`PanamaVectorConstants.HAS_FAST_INTEGER_VECTORS`, `VECTOR_BITSIZE`, the
/// minimum lengths, `Constants.HAS_FAST_VECTOR_FMA` for the vector `fma`,
/// `Constants.HAS_FAST_SCALAR_FMA` for the scalar tails and
/// `Constants.HAS_FAST_COMPRESS_MASK_CAST` for `filterByScore`), the species
/// lane counts, the loop bounds, the unrolling, the per-lane accumulators and
/// the scalar tails. The vectors are fixed-width lane arrays
/// ([`FloatVector`]) whose lanes compute what Java's lanes compute, and the
/// float reductions use the order HotSpot's C2 uses
/// ([`FloatVector::reduce_add`]), so float results are Java's bit for bit
/// under the same gate values.
///
/// # Divergence from Lucene 10.5.0: the overloads unify
///
/// Java declares each memory-segment kernel twice more, once taking
/// `(byte[], MemorySegment)` and once `(MemorySegment, MemorySegment)`, because
/// a `MemorySegment` is not a `byte[]`. In Rust both a heap vector and a
/// [`MemorySegment`](crate::store::MemorySegment)'s
/// [`bytes()`](crate::store::MemorySegment::bytes) are a `&[u8]`, so the two
/// overloads collapse into the single function named after the Java operation.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanamaVectorUtilSupport;

impl PanamaVectorUtilSupport {
    /// Creates the (stateless) support.
    ///
    /// Equivalent to the package-private `PanamaVectorUtilSupport()`
    /// constructor.
    pub const fn new() -> Self {
        Self
    }

    /// Returns the dot product computed over signed bytes.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.dotProduct(byte[], MemorySegment)`
    /// and `dotProduct(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        Ok(dot_product_body(a, b, true))
    }

    /// Returns the dot product computed as though the bytes were unsigned.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.uint8DotProduct(byte[], MemorySegment)`
    /// and `uint8DotProduct(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn uint8_dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        Ok(dot_product_body(a, b, false))
    }

    /// Returns the dot product computed over unsigned half-bytes, both
    /// uncompressed.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.int4DotProduct(byte[], MemorySegment)`
    /// and `int4DotProduct(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn int4_dot_product(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut i = 0;
        let mut res = 0i32;
        if a.len() >= 32 {
            i += loop_bound(a.len(), int4_constants().0);
            res = res.wrapping_add(int4_lanes(i, |e| {
                let p0 = mul8(a[e], b[e]);
                let p1 = mul8(a[e + 1], b[e + 1]);
                // prod16 >>> 8 is the odd byte, prod16 & 0xFF the even one.
                (i16::from(p1), i16::from(p0))
            }));
        }
        for k in i..a.len() {
            res = res.wrapping_add(i32::from(a[k] as i8) * i32::from(b[k] as i8));
        }
        Ok(res)
    }

    /// Returns the dot product computed over unsigned half-bytes, one
    /// compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4DotProductSinglePacked(byte[], MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when `unpacked` is
    /// shorter than twice the length of `packed`.
    pub fn int4_dot_product_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        check_single_packed(unpacked, packed)?;
        let plen = packed.len();
        let mut i = 0;
        let mut res = 0i32;
        if plen >= 32 {
            i += loop_bound(plen, int4_constants().0);
            res = res.wrapping_add(int4_lanes(i, |e| {
                let x = |k: usize| u16::from(mul8(packed[k] & 0x0F, unpacked[k + plen]));
                let y = |k: usize| u16::from(mul8(lshr4(packed[k]), unpacked[k]));
                ((x(e + 1) + x(e)) as i16, (y(e + 1) + y(e)) as i16)
            }));
        }
        for k in i..plen {
            let packed_byte = packed[k];
            let unpacked1 = i32::from(unpacked[k] as i8);
            let unpacked2 = i32::from(unpacked[k + plen] as i8);
            res = res.wrapping_add(i32::from(packed_byte & 0x0F) * unpacked2);
            res = res.wrapping_add(i32::from(packed_byte >> 4) * unpacked1);
        }
        Ok(res)
    }

    /// Returns the dot product computed over unsigned half-bytes, both
    /// compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4DotProductBothPacked(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn int4_dot_product_both_packed(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut i = 0;
        let mut res = 0i32;
        if a.len() >= 32 {
            i += loop_bound(a.len(), int4_constants().0);
            res = res.wrapping_add(int4_lanes(i, |e| {
                let x = |k: usize| u16::from(mul8(b[k] & 0x0F, a[k] & 0x0F));
                let y = |k: usize| u16::from(mul8(lshr4(b[k]), lshr4(a[k])));
                ((x(e + 1) + x(e)) as i16, (y(e + 1) + y(e)) as i16)
            }));
        }
        for k in i..a.len() {
            let a_byte = a[k];
            let b_byte = b[k];
            res = res.wrapping_add(i32::from(a_byte & 0x0F) * i32::from(b_byte & 0x0F));
            res = res.wrapping_add(i32::from(a_byte >> 4) * i32::from(b_byte >> 4));
        }
        Ok(res)
    }

    /// Returns the cosine similarity between two byte vectors.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.cosine(byte[], MemorySegment)` and
    /// `cosine(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn cosine(a: &[u8], b: &[u8]) -> Result<f32> {
        check_same_dimensions(a, b)?;
        let len = a.len();
        let mut i = 0;
        let mut sum = 0i32;
        let mut norm1 = 0i32;
        let mut norm2 = 0i32;
        if len >= 16 && PanamaVectorConstants::has_fast_integer_vectors() {
            let bits = vector_bitsize();
            // (bytes per lane computation, conversion through shorts)
            let (limit, through_short) = if bits >= 512 {
                (loop_bound(len, float_lanes()), true)
            } else if bits == 256 {
                (loop_bound(len, 8), false)
            } else {
                // Java's "overlapping read": 8 bytes loaded, the first 4 used.
                (loop_bound(len - 8, 8), true)
            };
            for k in 0..limit {
                let (x, y) = if through_short {
                    let x = b2s(a[k], true);
                    let y = b2s(b[k], true);
                    (
                        s2i(x.wrapping_mul(y), true),
                        (s2i(x.wrapping_mul(x), true), s2i(y.wrapping_mul(y), true)),
                    )
                } else {
                    let x = b2i(a[k], true);
                    let y = b2i(b[k], true);
                    (x * y, (x * x, y * y))
                };
                sum = sum.wrapping_add(x);
                norm1 = norm1.wrapping_add(y.0);
                norm2 = norm2.wrapping_add(y.1);
            }
            i = limit;
        }
        for k in i..len {
            let elem1 = i32::from(a[k] as i8);
            let elem2 = i32::from(b[k] as i8);
            sum = sum.wrapping_add(elem1 * elem2);
            norm1 = norm1.wrapping_add(elem1 * elem1);
            norm2 = norm2.wrapping_add(elem2 * elem2);
        }
        Ok((f64::from(sum) / (f64::from(norm1) * f64::from(norm2)).sqrt()) as f32)
    }

    /// Returns the sum of squared differences of two signed byte vectors.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.squareDistance(byte[], MemorySegment)`
    /// and `squareDistance(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        Ok(square_distance_body(a, b, true))
    }

    /// Returns the sum of squared differences of two unsigned byte vectors.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.uint8SquareDistance(byte[], MemorySegment)`
    /// and `uint8SquareDistance(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn uint8_square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        Ok(square_distance_body(a, b, false))
    }

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, both uncompressed.
    ///
    /// Equivalent to `PanamaVectorUtilSupport.int4SquareDistance(byte[], MemorySegment)`
    /// and `int4SquareDistance(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn int4_square_distance(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut i = 0;
        let mut res = 0i32;
        if a.len() >= 32 {
            i += loop_bound(a.len(), int4_constants().0);
            res = res.wrapping_add(int4_lanes(i, |e| {
                // diff8 = |b - a| per byte; the even byte feeds acc0, the odd acc1.
                let d = |k: usize| i16::from((b[k].wrapping_sub(a[k]) as i8).wrapping_abs() as u8);
                let d0 = d(e);
                let d1 = d(e + 1);
                (d0.wrapping_mul(d0), d1.wrapping_mul(d1))
            }));
        }
        for k in i..a.len() {
            let diff = i32::from(a[k] as i8) - i32::from(b[k] as i8);
            res = res.wrapping_add(diff * diff);
        }
        Ok(res)
    }

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, one compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4SquareDistanceSinglePacked(byte[], MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when `unpacked` is
    /// shorter than twice the length of `packed`.
    pub fn int4_square_distance_single_packed(unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        check_single_packed(unpacked, packed)?;
        let plen = packed.len();
        let mut i = 0;
        let mut res = 0i32;
        if plen >= 32 {
            i += loop_bound(plen, int4_constants().0);
            res = res.wrapping_add(int4_lanes(i, |e| {
                let x = |k: usize| {
                    let diff8 = (packed[k] & 0x0F).wrapping_sub(unpacked[k + plen]);
                    u16::from(mul8(diff8, diff8))
                };
                let y = |k: usize| {
                    let diff8a = lshr4(packed[k]).wrapping_sub(unpacked[k]);
                    u16::from(mul8(diff8a, diff8a))
                };
                ((x(e) + x(e + 1)) as i16, (y(e) + y(e + 1)) as i16)
            }));
        }
        for k in i..plen {
            let packed_byte = packed[k];
            let unpacked1 = i32::from(unpacked[k] as i8);
            let unpacked2 = i32::from(unpacked[k + plen] as i8);
            let diff1 = i32::from(packed_byte & 0x0F) - unpacked2;
            let diff2 = i32::from(packed_byte >> 4) - unpacked1;
            res = res.wrapping_add(diff1 * diff1 + diff2 * diff2);
        }
        Ok(res)
    }

    /// Returns the sum of squared differences between two unsigned half-byte
    /// vectors, both compressed.
    ///
    /// Equivalent to
    /// `PanamaVectorUtilSupport.int4SquareDistanceBothPacked(MemorySegment, MemorySegment)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneErrorKind::IllegalArgument`] when the lengths differ.
    pub fn int4_square_distance_both_packed(a: &[u8], b: &[u8]) -> Result<i32> {
        check_same_dimensions(a, b)?;
        let mut i = 0;
        let mut res = 0i32;
        if a.len() >= 32 {
            i += loop_bound(a.len(), int4_constants().0);
            res = res.wrapping_add(int4_lanes(i, |e| {
                let x = |k: usize| {
                    let diff8 = (b[k] & 0x0F).wrapping_sub(a[k] & 0x0F);
                    u16::from(mul8(diff8, diff8))
                };
                let y = |k: usize| {
                    let diff8a = lshr4(b[k]).wrapping_sub(lshr4(a[k]));
                    u16::from(mul8(diff8a, diff8a))
                };
                ((x(e) + x(e + 1)) as i16, (y(e) + y(e + 1)) as i16)
            }));
        }
        for k in i..a.len() {
            let a_byte = a[k];
            let b_byte = b[k];
            let diff1 = i32::from(a_byte & 0x0F) - i32::from(b_byte & 0x0F);
            let diff2 = i32::from(a_byte >> 4) - i32::from(b_byte >> 4);
            res = res.wrapping_add(diff1 * diff1 + diff2 * diff2);
        }
        Ok(res)
    }
}

/// `PanamaVectorUtilSupport.dotProductBody(ByteVectorLoader, ByteVectorLoader, boolean)`
/// with its 512-, 256- and 128-bit bodies.
fn dot_product_body(a: &[u8], b: &[u8], signed: bool) -> i32 {
    let len = a.len();
    let mut i = 0;
    let mut res = 0i32;
    if len >= 16 && PanamaVectorConstants::has_fast_integer_vectors() {
        let bits = vector_bitsize();
        if bits >= 512 {
            // dotProductBody512: 16-bit multiply, 32-bit add.
            i += loop_bound(len, float_lanes());
            for k in 0..i {
                let prod16 = b2s(a[k], signed).wrapping_mul(b2s(b[k], signed));
                res = res.wrapping_add(s2i(prod16, signed));
            }
        } else if bits == 256 {
            // dotProductBody256: 32-bit multiply and add.
            i += loop_bound(len, 8);
            for k in 0..i {
                res = res.wrapping_add(b2i(a[k], signed).wrapping_mul(b2i(b[k], signed)));
            }
        } else {
            // dotProductBody128: "overlapping read", 4 bytes per iteration.
            i += loop_bound(len - 8, 8);
            for k in 0..i {
                let prod16 = b2s(a[k], signed).wrapping_mul(b2s(b[k], signed));
                res = res.wrapping_add(s2i(prod16, signed));
            }
        }
    }
    if signed {
        for k in i..len {
            res = res.wrapping_add(i32::from(a[k] as i8) * i32::from(b[k] as i8));
        }
    } else {
        for k in i..len {
            res = res.wrapping_add(i32::from(a[k]) * i32::from(b[k]));
        }
    }
    res
}

/// `PanamaVectorUtilSupport.squareDistanceBody(ByteVectorLoader, ByteVectorLoader, boolean)`
/// with its 256+- and 128-bit bodies.
fn square_distance_body(a: &[u8], b: &[u8], signed: bool) -> i32 {
    let len = a.len();
    let mut i = 0;
    let mut res = 0i32;
    if len >= 16 && PanamaVectorConstants::has_fast_integer_vectors() {
        if vector_bitsize() >= 256 {
            // squareDistanceBody256: 32-bit sub, multiply and add.
            i += loop_bound(len, float_lanes());
            for k in 0..i {
                let diff32 = b2i(a[k], signed) - b2i(b[k], signed);
                res = res.wrapping_add(diff32 * diff32);
            }
        } else {
            // squareDistanceBody128: 16-bit sub, 32-bit multiply and add.
            i += loop_bound(len, 8);
            for k in 0..i {
                let diff16 = b2s(a[k], signed).wrapping_sub(b2s(b[k], signed));
                let diff32 = s2i(diff16, true);
                res = res.wrapping_add(diff32.wrapping_mul(diff32));
            }
        }
    }
    if signed {
        for k in i..len {
            let diff = i32::from(a[k] as i8) - i32::from(b[k] as i8);
            res = res.wrapping_add(diff * diff);
        }
    } else {
        for k in i..len {
            let diff = i32::from(a[k]) - i32::from(b[k]);
            res = res.wrapping_add(diff * diff);
        }
    }
    res
}

/// `PanamaVectorUtilSupport.dotProductBody(float[], float[], int)`: four
/// accumulators, unrolled 4x, then a one-vector tail.
fn dot_product_body_f32<const N: usize>(a: &[f32], b: &[f32], limit: usize) -> f32 {
    let mut i = 0usize;
    let mut acc1 = FloatVector::<N>::zero();
    let mut acc2 = FloatVector::<N>::zero();
    let mut acc3 = FloatVector::<N>::zero();
    let mut acc4 = FloatVector::<N>::zero();
    let unrolled_limit = limit as isize - 3 * N as isize;
    while (i as isize) < unrolled_limit {
        acc1 = fma_vec(
            FloatVector::from_array(a, i),
            FloatVector::from_array(b, i),
            acc1,
        );
        acc2 = fma_vec(
            FloatVector::from_array(a, i + N),
            FloatVector::from_array(b, i + N),
            acc2,
        );
        acc3 = fma_vec(
            FloatVector::from_array(a, i + 2 * N),
            FloatVector::from_array(b, i + 2 * N),
            acc3,
        );
        acc4 = fma_vec(
            FloatVector::from_array(a, i + 3 * N),
            FloatVector::from_array(b, i + 3 * N),
            acc4,
        );
        i += 4 * N;
    }
    while i < limit {
        acc1 = fma_vec(
            FloatVector::from_array(a, i),
            FloatVector::from_array(b, i),
            acc1,
        );
        i += N;
    }
    let res1 = acc1.add(acc2);
    let res2 = acc3.add(acc4);
    res1.add(res2).reduce_add()
}

/// `PanamaVectorUtilSupport.cosineBody(float[], float[], int)`: two sets of
/// accumulators, unrolled 2x, then a one-vector tail.
fn cosine_body_f32<const N: usize>(a: &[f32], b: &[f32], limit: usize) -> [f32; 3] {
    let mut i = 0usize;
    let mut sum1 = FloatVector::<N>::zero();
    let mut sum2 = FloatVector::<N>::zero();
    let mut norm1_1 = FloatVector::<N>::zero();
    let mut norm1_2 = FloatVector::<N>::zero();
    let mut norm2_1 = FloatVector::<N>::zero();
    let mut norm2_2 = FloatVector::<N>::zero();
    let unrolled_limit = limit as isize - N as isize;
    while (i as isize) < unrolled_limit {
        let va = FloatVector::from_array(a, i);
        let vb = FloatVector::from_array(b, i);
        sum1 = fma_vec(va, vb, sum1);
        norm1_1 = fma_vec(va, va, norm1_1);
        norm2_1 = fma_vec(vb, vb, norm2_1);

        let vc = FloatVector::from_array(a, i + N);
        let vd = FloatVector::from_array(b, i + N);
        sum2 = fma_vec(vc, vd, sum2);
        norm1_2 = fma_vec(vc, vc, norm1_2);
        norm2_2 = fma_vec(vd, vd, norm2_2);
        i += 2 * N;
    }
    while i < limit {
        let va = FloatVector::from_array(a, i);
        let vb = FloatVector::from_array(b, i);
        sum1 = fma_vec(va, vb, sum1);
        norm1_1 = fma_vec(va, va, norm1_1);
        norm2_1 = fma_vec(vb, vb, norm2_1);
        i += N;
    }
    [
        sum1.add(sum2).reduce_add(),
        norm1_1.add(norm1_2).reduce_add(),
        norm2_1.add(norm2_2).reduce_add(),
    ]
}

/// `PanamaVectorUtilSupport.squareDistanceBody(float[], float[], int)`.
fn square_distance_body_f32<const N: usize>(a: &[f32], b: &[f32], limit: usize) -> f32 {
    let mut i = 0usize;
    let mut acc1 = FloatVector::<N>::zero();
    let mut acc2 = FloatVector::<N>::zero();
    let mut acc3 = FloatVector::<N>::zero();
    let mut acc4 = FloatVector::<N>::zero();
    let unrolled_limit = limit as isize - 3 * N as isize;
    while (i as isize) < unrolled_limit {
        let diff1 = FloatVector::<N>::from_array(a, i).sub(FloatVector::from_array(b, i));
        acc1 = fma_vec(diff1, diff1, acc1);
        let diff2 = FloatVector::<N>::from_array(a, i + N).sub(FloatVector::from_array(b, i + N));
        acc2 = fma_vec(diff2, diff2, acc2);
        let diff3 =
            FloatVector::<N>::from_array(a, i + 2 * N).sub(FloatVector::from_array(b, i + 2 * N));
        acc3 = fma_vec(diff3, diff3, acc3);
        let diff4 =
            FloatVector::<N>::from_array(a, i + 3 * N).sub(FloatVector::from_array(b, i + 3 * N));
        acc4 = fma_vec(diff4, diff4, acc4);
        i += 4 * N;
    }
    while i < limit {
        let diff = FloatVector::<N>::from_array(a, i).sub(FloatVector::from_array(b, i));
        acc1 = fma_vec(diff, diff, acc1);
        i += N;
    }
    let res1 = acc1.add(acc2);
    let res2 = acc3.add(acc4);
    res1.add(res2).reduce_add()
}

/// The vector loop of `PanamaVectorUtilSupport.minMaxScalarQuantize`; returns
/// the reduced correction and the first index left to the scalar quantizer.
fn min_max_scalar_quantize_body<const N: usize>(
    vector: &[f32],
    dest: &mut [u8],
    scale: f32,
    alpha: f32,
    min_quantile: f32,
    max_quantile: f32,
) -> (f32, usize) {
    let mut sum = FloatVector::<N>::zero();
    let bound = loop_bound(vector.len(), N);
    let mut i = 0;
    while i < bound {
        let v = FloatVector::<N>::from_array(vector, i);
        let dxc = v
            .min_scalar(max_quantile)
            .max_scalar(min_quantile)
            .sub(FloatVector::broadcast(min_quantile));
        let t = fma_vec(
            dxc,
            FloatVector::broadcast(scale),
            FloatVector::broadcast(0.5f32),
        );
        // F2I, then castShape(BYTE_SPECIES) (I2B) into dest and castShape(FLOAT_SPECIES) (I2F).
        let mut dxq = [0.0f32; N];
        for k in 0..N {
            let rounded = t.0[k] as i32;
            dest[i + k] = rounded as u8;
            dxq[k] = rounded as f32;
        }
        let dxq = FloatVector(dxq).mul(FloatVector::broadcast(alpha));
        sum = fma_vec(
            v.sub(FloatVector::broadcast(min_quantile / 2f32)),
            FloatVector::broadcast(min_quantile),
            fma_vec(
                v.sub(FloatVector::broadcast(min_quantile)).sub(dxq),
                dxq,
                sum,
            ),
        );
        i += N;
    }
    (sum.reduce_add(), i)
}

/// The vector loop of `PanamaVectorUtilSupport.recalculateScalarQuantizationOffset`.
#[allow(clippy::too_many_arguments)]
fn recalculate_offset_body<const N: usize>(
    vector: &[u8],
    old_alpha: f32,
    old_min_quantile: f32,
    scale: f32,
    alpha: f32,
    min_quantile: f32,
    max_quantile: f32,
) -> (f32, usize) {
    let mut sum = FloatVector::<N>::zero();
    let bound = loop_bound(vector.len(), N);
    let mut i = 0;
    while i < bound {
        // ZERO_EXTEND_B2S, then castShape(FLOAT_SPECIES).
        let mut fv = [0.0f32; N];
        for k in 0..N {
            fv[k] = f32::from(vector[i + k]);
        }
        let fv = FloatVector(fv);
        let v = fma_vec(
            fv,
            FloatVector::broadcast(old_alpha),
            FloatVector::broadcast(old_min_quantile),
        );
        let dxc = v
            .min_scalar(max_quantile)
            .max_scalar(min_quantile)
            .sub(FloatVector::broadcast(min_quantile));
        let t = fma_vec(
            dxc,
            FloatVector::broadcast(scale),
            FloatVector::broadcast(0.5f32),
        );
        // F2I, then castShape(FLOAT_SPECIES) (I2F).
        let dxq = FloatVector(t.0.map(|x| (x as i32) as f32)).mul(FloatVector::broadcast(alpha));
        sum = fma_vec(
            v.sub(FloatVector::broadcast(min_quantile / 2f32)),
            FloatVector::broadcast(min_quantile),
            fma_vec(
                v.sub(FloatVector::broadcast(min_quantile)).sub(dxq),
                dxq,
                sum,
            ),
        );
        i += N;
    }
    (sum.reduce_add(), i)
}

/// `PanamaVectorUtilSupport.int4BitDotProduct256WithOffset` and
/// `int4BitDotProduct128WithOffset`.
///
/// Both sum the population counts of `q & d` per query stripe, 256- or
/// 128-bit vectors at a time, then byte by byte; the lane sums are exact
/// integers, so the grouping does not change the result and one loop serves
/// both bodies.
fn int4_bit_dot_product_with_offset(
    q: &[u8],
    d: &[u8],
    d_offset: usize,
    stripe_size: usize,
) -> i64 {
    let mut sub = [0i64; 4];
    for i in 0..stripe_size {
        let dv = d[d_offset + i];
        for (s, sub_ret) in sub.iter_mut().enumerate() {
            *sub_ret += i64::from((q[i + s * stripe_size] & dv).count_ones());
        }
    }
    sub[0] + (sub[1] << 1) + (sub[2] << 2) + (sub[3] << 3)
}

impl VectorUtilSupport for PanamaVectorUtilSupport {
    fn dot_product_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        check_same_dimensions_f32(a, b)?;
        let lanes = float_lanes();
        let mut i = 0;
        let mut res = 0.0f32;
        // if the array size is large (> 2x platform vector size), it's worth
        // the overhead to vectorize
        if a.len() > 2 * lanes {
            i += loop_bound(a.len(), lanes);
            res += with_float_lanes!(lanes, dot_product_body_f32(a, b, i));
        }
        for k in i..a.len() {
            res = fma(a[k], b[k], res);
        }
        Ok(res)
    }

    fn cosine_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        check_same_dimensions_f32(a, b)?;
        let lanes = float_lanes();
        let mut i = 0;
        let mut sum = 0.0f32;
        let mut norm1 = 0.0f32;
        let mut norm2 = 0.0f32;
        if a.len() > 2 * lanes {
            i += loop_bound(a.len(), lanes);
            let ret = with_float_lanes!(lanes, cosine_body_f32(a, b, i));
            sum += ret[0];
            norm1 += ret[1];
            norm2 += ret[2];
        }
        for k in i..a.len() {
            sum = fma(a[k], b[k], sum);
            norm1 = fma(a[k], a[k], norm1);
            norm2 = fma(b[k], b[k], norm2);
        }
        Ok((f64::from(sum) / (f64::from(norm1) * f64::from(norm2)).sqrt()) as f32)
    }

    fn square_distance_f32(&self, a: &[f32], b: &[f32]) -> Result<f32> {
        check_same_dimensions_f32(a, b)?;
        let lanes = float_lanes();
        let mut i = 0;
        let mut res = 0.0f32;
        if a.len() > 2 * lanes {
            i += loop_bound(a.len(), lanes);
            res += with_float_lanes!(lanes, square_distance_body_f32(a, b, i));
        }
        for k in i..a.len() {
            let diff = a[k] - b[k];
            res = fma(diff, diff, res);
        }
        Ok(res)
    }

    fn dot_product_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::dot_product(a, b)
    }

    fn int4_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_dot_product(a, b)
    }

    fn int4_dot_product_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        Self::int4_dot_product_single_packed(unpacked, packed)
    }

    fn int4_dot_product_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_dot_product_both_packed(a, b)
    }

    fn uint8_dot_product(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::uint8_dot_product(a, b)
    }

    fn cosine_bytes(&self, a: &[u8], b: &[u8]) -> Result<f32> {
        Self::cosine(a, b)
    }

    fn square_distance_bytes(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::square_distance(a, b)
    }

    fn int4_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_square_distance(a, b)
    }

    fn int4_square_distance_single_packed(&self, unpacked: &[u8], packed: &[u8]) -> Result<i32> {
        Self::int4_square_distance_single_packed(unpacked, packed)
    }

    fn int4_square_distance_both_packed(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::int4_square_distance_both_packed(a, b)
    }

    fn uint8_square_distance(&self, a: &[u8], b: &[u8]) -> Result<i32> {
        Self::uint8_square_distance(a, b)
    }

    fn find_next_geq(&self, buffer: &[i32], target: i32, from: usize, to: usize) -> usize {
        let lanes = PanamaVectorConstants::int_lanes();
        // ENABLE_FIND_NEXT_GEQ_VECTOR_OPTO: at least 8 int lanes, or aarch64.
        let enable = lanes >= 8 || Constants::os_arch() == "aarch64";
        let mut from = from;
        if enable {
            // The V1 intersection algorithm of Lemire, Boytsov and Kurz
            // (<https://arxiv.org/pdf/1401.6399>) with T = the int lane count.
            while from + lanes < to {
                if buffer[from + lanes] >= target {
                    let true_count = buffer[from..from + lanes]
                        .iter()
                        .filter(|&&v| v < target)
                        .count();
                    return from + true_count;
                }
                from += lanes + 1;
            }
        }
        for (i, &value) in buffer.iter().enumerate().take(to).skip(from) {
            if value >= target {
                return i;
            }
        }
        to
    }

    fn int4_bit_dot_product(&self, int4_quantized: &[u8], binary_quantized: &[u8]) -> Result<i64> {
        let (q, d) = (int4_quantized, binary_quantized);
        debug_assert!(q.len() == d.len() * 4);
        // 128 / 8 == 16
        if d.len() >= 16 && PanamaVectorConstants::has_fast_integer_vectors() {
            let bits = vector_bitsize();
            if bits >= 256 || bits == 128 {
                return Ok(int4_bit_dot_product_with_offset(q, d, 0, d.len()));
            }
        }
        DefaultVectorUtilSupport::int4_bit_dot_product_impl(q, d)
    }

    fn int4_dibit_dot_product(&self, int4_quantized: &[u8], dibit_quantized: &[u8]) -> Result<i64> {
        let (q, d) = (int4_quantized, dibit_quantized);
        debug_assert!(q.len() == d.len() * 2);
        let stripe_size = d.len() / 2;
        if stripe_size >= 16 {
            let bits = vector_bitsize();
            if bits >= 256 || bits == 128 {
                let ret0 = int4_bit_dot_product_with_offset(q, d, 0, stripe_size);
                let ret1 = int4_bit_dot_product_with_offset(q, d, stripe_size, stripe_size);
                return Ok(ret0 + (ret1 << 1));
            }
        }
        DefaultVectorUtilSupport::int4_dibit_dot_product_impl(q, d)
    }

    fn min_max_scalar_quantize(
        &self,
        vector: &[f32],
        dest: &mut [u8],
        scale: f32,
        alpha: f32,
        min_quantile: f32,
        max_quantile: f32,
    ) -> Result<f32> {
        if vector.len() != dest.len() {
            return Err(LuceneError::illegal_argument(format!(
                "vector dimensions differ: {}!={}",
                vector.len(),
                dest.len()
            )));
        }
        let mut correction = 0.0f32;
        let mut i = 0;
        // only vectorize if we have a viable BYTE_SPECIES we can use for output
        if vector_bitsize() >= 256 {
            (correction, i) = with_float_lanes!(
                float_lanes(),
                min_max_scalar_quantize_body(
                    vector,
                    dest,
                    scale,
                    alpha,
                    min_quantile,
                    max_quantile
                )
            );
        }
        correction += ScalarQuantizer::new(alpha, scale, min_quantile, max_quantile)
            .quantize(vector, dest, i);
        Ok(correction)
    }

    fn recalculate_scalar_quantization_offset(
        &self,
        vector: &[u8],
        old_alpha: f32,
        old_min_quantile: f32,
        scale: f32,
        alpha: f32,
        min_quantile: f32,
        max_quantile: f32,
    ) -> f32 {
        let mut correction = 0.0f32;
        let mut i = 0;
        if vector_bitsize() >= 256 {
            (correction, i) = with_float_lanes!(
                float_lanes(),
                recalculate_offset_body(
                    vector,
                    old_alpha,
                    old_min_quantile,
                    scale,
                    alpha,
                    min_quantile,
                    max_quantile
                )
            );
        }
        correction += ScalarQuantizer::new(alpha, scale, min_quantile, max_quantile)
            .recalculate_offset(vector, i, old_alpha, old_min_quantile);
        correction
    }

    fn filter_by_score(
        &self,
        doc_buffer: &mut [i32],
        score_buffer: &mut [f64],
        min_score_inclusive: f64,
        up_to: usize,
    ) -> usize {
        let mut new_upto = 0usize;
        let mut i = 0usize;
        if *HAS_FAST_COMPRESS_MASK_CAST {
            // DOUBLE_SPECIES and INT_FOR_DOUBLE_SPECIES have the long lane count.
            let lanes = PanamaVectorConstants::long_lanes();
            let bound = loop_bound(up_to, lanes);
            while i < bound {
                // compress(mask): the selected lanes move down in order, the
                // others become zero; intoArray then stores all the lanes.
                let mut scores = [0.0f64; 32];
                let mut docs = [0i32; 32];
                let mut true_count = 0;
                for k in 0..lanes {
                    let score = score_buffer[i + k];
                    if score >= min_score_inclusive {
                        scores[true_count] = score;
                        docs[true_count] = doc_buffer[i + k];
                        true_count += 1;
                    }
                }
                score_buffer[new_upto..new_upto + lanes].copy_from_slice(&scores[..lanes]);
                doc_buffer[new_upto..new_upto + lanes].copy_from_slice(&docs[..lanes]);
                new_upto += true_count;
                i += lanes;
            }
        }
        while i < up_to {
            let doc = doc_buffer[i];
            let score = score_buffer[i];
            doc_buffer[new_upto] = doc;
            score_buffer[new_upto] = score;
            if score >= min_score_inclusive {
                new_upto += 1;
            }
            i += 1;
        }
        new_upto
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reduce_add_halves_the_lanes() {
        // Values chosen so that every other order rounds differently.
        let v = FloatVector([1.0e8f32, 1.0, -1.0e8, 3.0, 5.0, 7.0, 0.5, -2.0]);
        let expected = ((1.0e8f32 + 5.0) + (-1.0e8 + 0.5)) + ((1.0 + 7.0) + (3.0 + -2.0));
        assert_eq!(v.reduce_add().to_bits(), expected.to_bits());
        assert_eq!(
            FloatVector([-0.0f32; 8]).reduce_add().to_bits(),
            (-0.0f32).to_bits()
        );
    }

    #[test]
    fn byte_kernels_equal_the_scalar_ones_on_int4_range() {
        let s = DefaultVectorUtilSupport::new();
        let p = PanamaVectorUtilSupport::new();
        for len in [0usize, 1, 15, 16, 17, 31, 32, 33, 64, 100, 4097, 8200] {
            let a: Vec<u8> = (0..len).map(|i| ((i * 7 + 3) % 16) as u8).collect();
            let b: Vec<u8> = (0..len).map(|i| ((i * 11 + 5) % 16) as u8).collect();
            assert_eq!(
                p.int4_dot_product(&a, &b).unwrap(),
                s.int4_dot_product(&a, &b).unwrap()
            );
            assert_eq!(
                p.int4_square_distance(&a, &b).unwrap(),
                s.int4_square_distance(&a, &b).unwrap()
            );
            assert_eq!(
                p.dot_product_bytes(&a, &b).unwrap(),
                s.dot_product_bytes(&a, &b).unwrap()
            );
            assert_eq!(
                p.uint8_dot_product(&a, &b).unwrap(),
                s.uint8_dot_product(&a, &b).unwrap()
            );
        }
    }

    #[test]
    fn filter_by_score_keeps_java_survivors() {
        let p = PanamaVectorUtilSupport::new();
        let mut docs: Vec<i32> = (0..21).collect();
        let mut scores: Vec<f64> = (0..21).map(|i| f64::from(i % 5)).collect();
        let n = p.filter_by_score(&mut docs, &mut scores, 2.0, 21);
        let kept: Vec<i32> = (0..21).filter(|i| i % 5 >= 2).collect();
        assert_eq!(n, kept.len());
        assert_eq!(&docs[..n], &kept[..]);
    }
}
