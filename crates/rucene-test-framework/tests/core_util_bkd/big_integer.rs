//! The members of `java.math.BigInteger` that `TestBKD` uses: the random
//! constructor `new BigInteger(numBits, rnd)`, `negate()`, `compareTo` and
//! `toByteArray()` (which `NumericUtils.bigIntToSortableBytes` consumes),
//! written from their Javadoc.

use std::cmp::Ordering;

use randomizedtesting::jdk::random::Random;

/// A `java.math.BigInteger`: a sign and a big-endian magnitude without
/// leading zero bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BigInteger {
    negative: bool,
    magnitude: Vec<u8>,
}

impl BigInteger {
    /// `new BigInteger(numBits, rnd)`: uniformly distributed over `[0,
    /// 2^numBits - 1]`; its bits are `numBits` bits of `rnd.nextBytes` over
    /// `ceil(numBits / 8)` bytes, the excess high bits of the first byte
    /// cleared.
    pub fn random(num_bits: i32, rnd: &dyn Random) -> BigInteger {
        let num_bytes = ((num_bits as i64 + 7) / 8) as usize;
        let mut random_bits = vec![0u8; num_bytes];
        if num_bytes > 0 {
            rnd.next_bytes(&mut random_bits);
            let excess_bits = 8 * num_bytes as i32 - num_bits;
            random_bits[0] &= ((1i32 << (8 - excess_bits)) - 1) as u8;
        }
        let first = random_bits
            .iter()
            .position(|&b| b != 0)
            .unwrap_or(random_bits.len());
        BigInteger {
            negative: false,
            magnitude: random_bits[first..].to_vec(),
        }
    }

    /// `negate()`.
    pub fn negate(&self) -> BigInteger {
        BigInteger {
            negative: !self.negative && !self.magnitude.is_empty(),
            magnitude: self.magnitude.clone(),
        }
    }

    /// `toByteArray()`: the two's-complement big-endian representation in
    /// the minimal number of bytes (at least one).
    pub fn to_byte_array(&self) -> Vec<u8> {
        // The magnitude with a leading zero byte, so its sign bit is clear.
        let mut bytes = Vec::with_capacity(self.magnitude.len() + 1);
        bytes.push(0u8);
        bytes.extend_from_slice(&self.magnitude);
        if self.negative {
            // Two's complement: invert and add one.
            for b in bytes.iter_mut() {
                *b = !*b;
            }
            for b in bytes.iter_mut().rev() {
                let (v, carry) = b.overflowing_add(1);
                *b = v;
                if !carry {
                    break;
                }
            }
            // Drop redundant leading 0xff bytes.
            let mut start = 0;
            while start + 1 < bytes.len() && bytes[start] == 0xff && bytes[start + 1] & 0x80 != 0 {
                start += 1;
            }
            bytes[start..].to_vec()
        } else {
            let mut start = 0;
            while start + 1 < bytes.len() && bytes[start] == 0 && bytes[start + 1] & 0x80 == 0 {
                start += 1;
            }
            bytes[start..].to_vec()
        }
    }

    /// The magnitude order of two magnitudes.
    fn compare_magnitude(a: &[u8], b: &[u8]) -> Ordering {
        a.len().cmp(&b.len()).then_with(|| a.cmp(b))
    }
}

impl std::fmt::Display for BigInteger {
    /// The value in base 16 (a test diagnostic only).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.negative {
            write!(f, "-")?;
        }
        write!(f, "0x")?;
        for b in &self.magnitude {
            write!(f, "{b:02x}")?;
        }
        Ok(())
    }
}

impl PartialOrd for BigInteger {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BigInteger {
    /// `compareTo(val)`.
    fn cmp(&self, other: &Self) -> Ordering {
        match (self.negative, other.negative) {
            (false, true) => Ordering::Greater,
            (true, false) => Ordering::Less,
            (false, false) => Self::compare_magnitude(&self.magnitude, &other.magnitude),
            (true, true) => Self::compare_magnitude(&other.magnitude, &self.magnitude),
        }
    }
}
