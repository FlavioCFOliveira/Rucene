//! Bit mixing utilities, ported from `org.apache.lucene.internal.hppc.BitMixer`.
//!
//! The purpose of these functions is to distribute a key space evenly over the
//! whole `i32` range. Lucene forked them from HPPC 0.10.0
//! (`com.carrotsearch.hppc.BitMixer`).
//!
//! # Adaptation
//!
//! Java overloads a single `mix` (and `mixPhi`) name for every primitive type.
//! Rust has no overloading, so each overload becomes a distinct function whose
//! suffix names the Java type it corresponds to:
//!
//! | Java                  | Rust                     |
//! |-----------------------|--------------------------|
//! | `mix(byte)`           | [`BitMixer::mix_i8`]     |
//! | `mix(short)`          | [`BitMixer::mix_i16`]    |
//! | `mix(char)`           | [`BitMixer::mix_u16`]    |
//! | `mix(int)`            | [`BitMixer::mix_i32`]    |
//! | `mix(long)`           | [`BitMixer::mix_i64`]    |
//! | `mix(float)`          | [`BitMixer::mix_f32`]    |
//! | `mix(double)`         | [`BitMixer::mix_f64`]    |
//! | `mix(Object)`         | [`BitMixer::mix_hash`]   |
//!
//! Java's `char` is a UTF-16 code unit, i.e. a `u16` in Rust, never a Rust
//! `char` (which is a Unicode scalar value and would hash differently outside
//! the Basic Multilingual Plane).
//!
//! `mix(Object)` cannot be reproduced literally because Rust has no universal
//! `Object.hashCode()`. [`BitMixer::mix_hash`] takes the already-computed
//! 32-bit hash instead, which is exactly what the Java method does with the
//! value returned by `hashCode()`.

use super::support::{double_to_long_bits, float_to_int_bits};

/// Golden ratio bit mixer constant for 32-bit keys (`BitMixer.PHI_C32`).
const PHI_C32: u32 = 0x9e37_79b9;

/// Golden ratio bit mixer constant for 64-bit keys (`BitMixer.PHI_C64`).
const PHI_C64: u64 = 0x9e37_79b9_7f4a_7c15;

/// Port of `org.apache.lucene.internal.hppc.BitMixer`.
///
/// A stateless namespace: Lucene's class is `final` and exposes only static
/// methods, so this port carries no data.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Default)]
pub struct BitMixer;

impl BitMixer {
    // -----------------------------------------------------------------------
    // Don't bother mixing very small key domains much.
    // -----------------------------------------------------------------------

    /// Equivalent of Java `BitMixer.mix(byte)`.
    ///
    /// Note that this is *not* the same as [`Self::mix_phi_i8`]: Lucene's
    /// `mix(byte)` deliberately omits the final xor-shift.
    #[inline]
    pub fn mix_i8(key: i8) -> i32 {
        ((key as i32 as u32).wrapping_mul(PHI_C32)) as i32
    }

    /// Equivalent of Java `BitMixer.mix(short)`.
    #[inline]
    pub fn mix_i16(key: i16) -> i32 {
        Self::mix_phi_i16(key)
    }

    /// Equivalent of Java `BitMixer.mix(char)`, taking a UTF-16 code unit.
    #[inline]
    pub fn mix_u16(key: u16) -> i32 {
        Self::mix_phi_u16(key)
    }

    // -----------------------------------------------------------------------
    // Better mix for larger key domains.
    // -----------------------------------------------------------------------

    /// Equivalent of Java `BitMixer.mix(int)`.
    #[inline]
    pub fn mix_i32(key: i32) -> i32 {
        Self::mix32(key)
    }

    /// Equivalent of Java `BitMixer.mix(float)`.
    #[inline]
    pub fn mix_f32(key: f32) -> i32 {
        Self::mix32(float_to_int_bits(key))
    }

    /// Equivalent of Java `BitMixer.mix(double)`.
    #[inline]
    pub fn mix_f64(key: f64) -> i32 {
        Self::mix64(double_to_long_bits(key)) as i32
    }

    /// Equivalent of Java `BitMixer.mix(long)`.
    #[inline]
    pub fn mix_i64(key: i64) -> i32 {
        Self::mix64(key) as i32
    }

    /// Equivalent of Java `BitMixer.mix(Object)`, given the object's hash.
    ///
    /// Java reads `key.hashCode()` and returns `0` for `null`; Rust has no
    /// universal hash method, so the caller supplies the 32-bit hash (and uses
    /// `0` for the absent value, exactly as Java does for `null`).
    #[inline]
    pub fn mix_hash(hash: i32) -> i32 {
        Self::mix32(hash)
    }

    /// MurmurHash3's plain finalization step. Equivalent of `BitMixer.mix32`.
    #[inline]
    pub fn mix32(k: i32) -> i32 {
        let mut k = k as u32;
        k = (k ^ (k >> 16)).wrapping_mul(0x85eb_ca6b);
        k = (k ^ (k >> 13)).wrapping_mul(0xc2b2_ae35);
        (k ^ (k >> 16)) as i32
    }

    /// David Stafford's variant 9 of the 64-bit mix function, i.e. the
    /// MurmurHash3 finalization step with different shifts and constants.
    ///
    /// Equivalent of `BitMixer.mix64`. Variant 9 is picked because it contains
    /// two 32-bit shifts, which can be optimised into better machine code.
    ///
    /// See <http://zimbry.blogspot.com/2011/09/better-bit-mixing-improving-on.html>.
    #[inline]
    pub fn mix64(z: i64) -> i64 {
        let mut z = z as u64;
        z = (z ^ (z >> 32)).wrapping_mul(0x4cd6_944c_5cc2_0b6d);
        z = (z ^ (z >> 29)).wrapping_mul(0xfc12_c5b1_9d32_59e9);
        (z ^ (z >> 32)) as i64
    }

    // -----------------------------------------------------------------------
    // Golden ratio bit mixers.
    // -----------------------------------------------------------------------

    /// Equivalent of Java `BitMixer.mixPhi(byte)`.
    #[inline]
    pub fn mix_phi_i8(k: i8) -> i32 {
        Self::mix_phi_i32(k as i32)
    }

    /// Equivalent of Java `BitMixer.mixPhi(char)`, taking a UTF-16 code unit.
    #[inline]
    pub fn mix_phi_u16(k: u16) -> i32 {
        Self::mix_phi_i32(k as i32)
    }

    /// Equivalent of Java `BitMixer.mixPhi(short)`.
    #[inline]
    pub fn mix_phi_i16(k: i16) -> i32 {
        Self::mix_phi_i32(k as i32)
    }

    /// Equivalent of Java `BitMixer.mixPhi(int)`.
    #[inline]
    pub fn mix_phi_i32(k: i32) -> i32 {
        let h = (k as u32).wrapping_mul(PHI_C32);
        (h ^ (h >> 16)) as i32
    }

    /// Equivalent of Java `BitMixer.mixPhi(float)`.
    #[inline]
    pub fn mix_phi_f32(k: f32) -> i32 {
        let h = (float_to_int_bits(k) as u32).wrapping_mul(PHI_C32);
        (h ^ (h >> 16)) as i32
    }

    /// Equivalent of Java `BitMixer.mixPhi(double)`.
    #[inline]
    pub fn mix_phi_f64(k: f64) -> i32 {
        let h = (double_to_long_bits(k) as u64).wrapping_mul(PHI_C64);
        ((h ^ (h >> 32)) as u32) as i32
    }

    /// Equivalent of Java `BitMixer.mixPhi(long)`.
    #[inline]
    pub fn mix_phi_i64(k: i64) -> i32 {
        let h = (k as u64).wrapping_mul(PHI_C64);
        ((h ^ (h >> 32)) as u32) as i32
    }

    /// Equivalent of Java `BitMixer.mixPhi(Object)`, given the object's hash.
    ///
    /// Java computes `k.hashCode() * PHI_C32` (using `0` when `k` is `null`);
    /// the caller supplies that hash here, for the reason explained on
    /// [`Self::mix_hash`].
    #[inline]
    pub fn mix_phi_hash(hash: i32) -> i32 {
        let h = (hash as u32).wrapping_mul(PHI_C32);
        (h ^ (h >> 16)) as i32
    }
}
