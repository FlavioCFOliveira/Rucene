//! Small floating-point encoders ported from `org.apache.lucene.util.SmallFloat`.
//!
//! These helpers compress `f32` values into 8-bit floats and positive integers
//! into 4-bit or byte representations while preserving ordering.

#![deny(unsafe_code)]

use crate::error::LuceneError;

/// Floating point numbers smaller than 32 bits, equivalent to Lucene's
/// `SmallFloat`.
pub struct SmallFloat;

impl SmallFloat {
    /// Converts a 32-bit float to an 8-bit float.
    ///
    /// Values less than zero are mapped to zero. Values are truncated to the
    /// nearest representable 8-bit value. Values between zero and the smallest
    /// representable value are rounded up.
    pub fn float_to_byte(f: f32, num_mantissa_bits: i32, zero_exp: i32) -> u8 {
        let fzero = (63 - zero_exp) << num_mantissa_bits;
        let bits = f.to_bits() as i32;
        let smallfloat = bits >> (24 - num_mantissa_bits);
        if smallfloat <= fzero {
            if bits <= 0 {
                0 // negative numbers and zero both map to 0 byte
            } else {
                1 // underflow is mapped to smallest non-zero number
            }
        } else if smallfloat >= fzero + 0x100 {
            255 // overflow maps to largest number (Java byte -1 == 255 unsigned)
        } else {
            (smallfloat - fzero) as u8
        }
    }

    /// Converts an 8-bit float back to a 32-bit float.
    pub fn byte_to_float(b: u8, num_mantissa_bits: i32, zero_exp: i32) -> f32 {
        if b == 0 {
            return 0.0f32;
        }
        let mut bits = ((b as i32) & 0xff) << (24 - num_mantissa_bits);
        bits += (63 - zero_exp) << 24;
        f32::from_bits(bits as u32)
    }

    /// `floatToByte(b, mantissaBits=3, zeroExponent=15)`.
    pub fn float_to_byte315(f: f32) -> u8 {
        let bits = f.to_bits() as i32;
        let smallfloat = bits >> (24 - 3);
        let fzero = (63 - 15) << 3;
        if smallfloat <= fzero {
            if bits <= 0 {
                0
            } else {
                1
            }
        } else if smallfloat >= fzero + 0x100 {
            255
        } else {
            (smallfloat - fzero) as u8
        }
    }

    /// `byteToFloat(b, mantissaBits=3, zeroExponent=15)`.
    pub fn byte315_to_float(b: u8) -> f32 {
        if b == 0 {
            return 0.0f32;
        }
        let mut bits = ((b as i32) & 0xff) << (24 - 3);
        bits += (63 - 15) << 24;
        f32::from_bits(bits as u32)
    }

    /// Float-like encoding for positive longs that preserves ordering and 4
    /// significant bits.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `i` is negative.
    pub fn long_to_int4(i: i64) -> Result<i32, LuceneError> {
        if i < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "Only supports positive values, got {}",
                i
            )));
        }
        let num_bits = 64 - (i as u64).leading_zeros() as i32;
        if num_bits < 4 {
            Ok(i as i32)
        } else {
            let shift = num_bits - 4;
            let encoded = (i >> shift) as i32;
            let encoded = encoded & 0x07;
            Ok(encoded | ((shift + 1) << 3))
        }
    }

    /// Decodes a value encoded with [`Self::long_to_int4`].
    pub fn int4_to_long(i: i32) -> i64 {
        let bits = (i & 0x07) as i64;
        let shift = (i >> 3) - 1;
        if shift == -1 {
            bits
        } else {
            (bits | 0x08) << shift
        }
    }

    /// Encodes an integer to a byte, built on top of [`Self::long_to_int4`].
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `i` is negative.
    pub fn int_to_byte4(i: i32) -> Result<u8, LuceneError> {
        if i < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "Only supports positive values, got {}",
                i
            )));
        }
        if i < NUM_FREE_VALUES {
            Ok(i as u8)
        } else {
            let encoded =
                NUM_FREE_VALUES + Self::long_to_int4((i as i64) - NUM_FREE_VALUES as i64)?;
            Ok(encoded as u8)
        }
    }

    /// Decodes a value encoded with [`Self::int_to_byte4`].
    pub fn byte4_to_int(b: u8) -> i32 {
        let i = b as i32;
        if i < NUM_FREE_VALUES {
            i
        } else {
            (NUM_FREE_VALUES as i64 + Self::int4_to_long(i - NUM_FREE_VALUES)) as i32
        }
    }
}

const fn max_int4_for_integer_max() -> i32 {
    let i: i64 = i32::MAX as i64;
    let num_bits = 64 - (i as u64).leading_zeros() as i32;
    let shift = num_bits - 4;
    let encoded = (i >> shift) as i32;
    let encoded = encoded & 0x07;
    encoded | ((shift + 1) << 3)
}

/// Maximum value produced by `longToInt4(Integer.MAX_VALUE)`.
const MAX_INT4: i32 = max_int4_for_integer_max();
/// Number of byte values reserved for verbatim small integers.
const NUM_FREE_VALUES: i32 = 255 - MAX_INT4;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_to_byte315_round_trip() {
        // Values from the Lucene 10.5.0 SmallFloat javadoc for floatToByte315.
        assert_eq!(SmallFloat::float_to_byte315(0.0f32), 0);
        assert_eq!(SmallFloat::byte315_to_float(0), 0.0f32);

        let smallest = SmallFloat::byte315_to_float(1);
        assert_eq!(SmallFloat::float_to_byte315(smallest), 1);
        assert!(smallest > 0.0f32);

        let largest = SmallFloat::byte315_to_float(255);
        assert_eq!(SmallFloat::float_to_byte315(largest), 255);

        // Negative values map to zero; underflow maps to 1.
        assert_eq!(SmallFloat::float_to_byte315(-1.0f32), 0);
        assert_eq!(SmallFloat::float_to_byte315(1e-15f32), 1);
    }

    #[test]
    fn generic_float_byte_round_trip() {
        let values = [0.0f32, 1.0f32, 100.0f32, 0.5f32, 1e10f32];
        for &v in &values {
            let b = SmallFloat::float_to_byte(v, 5, 31);
            let dec = SmallFloat::byte_to_float(b, 5, 31);
            // Lossy encoding: the decoded value is <= the original and maps back
            // to the same byte.
            assert_eq!(SmallFloat::float_to_byte(dec, 5, 31), b);
        }
    }

    #[test]
    fn long_to_int4_ordering() {
        let values: Vec<i64> = (0..100).chain([255, 256, 1000, 10000, i64::MAX]).collect();
        let encoded: Vec<i32> = values
            .iter()
            .map(|&v| SmallFloat::long_to_int4(v).unwrap())
            .collect();
        assert!(encoded.windows(2).all(|w| w[0] <= w[1]));

        // Round-trip through int4ToLong is lossy; it must preserve the encoded value.
        for &v in &values {
            let enc = SmallFloat::long_to_int4(v).unwrap();
            let dec = SmallFloat::int4_to_long(enc);
            assert_eq!(SmallFloat::long_to_int4(dec).unwrap(), enc);
        }
    }

    #[test]
    fn int_to_byte4_round_trip() {
        let small_values = [0, 1, 10, 15, 16];
        for &v in &small_values {
            let b = SmallFloat::int_to_byte4(v).unwrap();
            assert_eq!(SmallFloat::byte4_to_int(b), v);
        }

        // Values >= NUM_FREE_VALUES are lossy; verify monotonicity and stable encoding.
        let lossy_values = [100, 1000, 1_000_000, i32::MAX];
        for &v in &lossy_values {
            let b = SmallFloat::int_to_byte4(v).unwrap();
            let decoded = SmallFloat::byte4_to_int(b);
            assert!(decoded <= v);
            assert_eq!(SmallFloat::int_to_byte4(decoded).unwrap(), b);
        }

        assert!(SmallFloat::int_to_byte4(-1).is_err());
    }
}
