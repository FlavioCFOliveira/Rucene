//! Port of `java.util.Random` of JDK 25: the 48-bit linear congruential
//! generator, written from the JDK source read as specification.

#![deny(unsafe_code)]

/// The `java.util.Random` linear congruential generator.
///
/// Equivalent to `java.util.Random` of JDK 25 (`Random(long)`, `setSeed`,
/// `next(int)`, `nextInt()`, `nextInt(int)`, `nextLong()`, `nextBoolean()`,
/// `nextFloat()`, `nextDouble()`): the same seed yields the same sequence as
/// the JDK. Lucene code that takes a `java.util.Random` uses this type.
///
/// **Divergence.** `java.util.Random` is thread-safe (its seed is an
/// `AtomicLong`); this type takes `&mut self`, which gives the same sequence
/// to a single owner.
#[derive(Debug, Clone)]
pub struct JavaRandom {
    seed: i64,
}

impl JavaRandom {
    const MULTIPLIER: i64 = 0x5_DEEC_E66D;
    const ADDEND: i64 = 0xB;
    const MASK: i64 = (1 << 48) - 1;
    /// `0x1.0p-53`.
    const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;
    /// `0x1.0p-24`.
    const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;

    /// Creates a generator with the given seed.
    ///
    /// Equivalent to `new java.util.Random(long)`.
    pub fn new(seed: i64) -> Self {
        Self {
            seed: Self::initial_scramble(seed),
        }
    }

    fn initial_scramble(seed: i64) -> i64 {
        (seed ^ Self::MULTIPLIER) & Self::MASK
    }

    /// Equivalent to `java.util.Random.setSeed(long)`.
    pub fn set_seed(&mut self, seed: i64) {
        self.seed = Self::initial_scramble(seed);
    }

    /// Generates the next pseudorandom number of `bits` random bits (1 to
    /// 32).
    ///
    /// Equivalent to the protected `java.util.Random.next(int)`.
    pub fn next_bits(&mut self, bits: u32) -> i32 {
        self.seed = self
            .seed
            .wrapping_mul(Self::MULTIPLIER)
            .wrapping_add(Self::ADDEND)
            & Self::MASK;
        ((self.seed as u64) >> (48 - bits)) as i32
    }

    /// Returns the next pseudorandom `int`.
    ///
    /// Equivalent to `java.util.Random.nextInt()`.
    pub fn next_int(&mut self) -> i32 {
        self.next_bits(32)
    }

    /// Returns the next pseudorandom `int` in `[0, bound)`.
    ///
    /// Equivalent to `java.util.Random.nextInt(int)`, whose rejection loop
    /// keeps the distribution uniform for a bound that is not a power of two.
    ///
    /// # Panics
    ///
    /// When `bound` is not positive, where Java throws
    /// `IllegalArgumentException("bound must be positive")`.
    pub fn next_int_bound(&mut self, bound: i32) -> i32 {
        assert!(bound > 0, "bound must be positive");
        let mut r = self.next_bits(31);
        let m = bound - 1;
        if (bound & m) == 0 {
            // i.e., bound is a power of 2
            r = ((bound as i64).wrapping_mul(i64::from(r)) >> 31) as i32;
        } else {
            let mut u = r;
            loop {
                r = u % bound;
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    break;
                }
                u = self.next_bits(31);
            }
        }
        r
    }

    /// Returns the next pseudorandom `long`.
    ///
    /// Equivalent to `java.util.Random.nextLong()`.
    pub fn next_long(&mut self) -> i64 {
        ((self.next_bits(32) as i64) << 32).wrapping_add(self.next_bits(32) as i64)
    }

    /// Returns the next pseudorandom `boolean`.
    ///
    /// Equivalent to `java.util.Random.nextBoolean()`.
    pub fn next_boolean(&mut self) -> bool {
        self.next_bits(1) != 0
    }

    /// Returns the next pseudorandom `float` in `[0, 1)`.
    ///
    /// Equivalent to `java.util.Random.nextFloat()`.
    pub fn next_float(&mut self) -> f32 {
        self.next_bits(24) as f32 * Self::FLOAT_UNIT
    }

    /// Returns the next pseudorandom `double` in `[0, 1)`.
    ///
    /// Equivalent to `java.util.Random.nextDouble()`.
    pub fn next_double(&mut self) -> f64 {
        (((self.next_bits(26) as i64) << 27) + self.next_bits(27) as i64) as f64 * Self::DOUBLE_UNIT
    }
}
