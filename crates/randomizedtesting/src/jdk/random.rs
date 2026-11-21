//! `java.util.Random` and the `java.util.random.RandomGenerator` defaults it
//! inherits, as of JDK 25.
//!
//! randomizedtesting's generators (`Xoroshiro128PlusRandom`,
//! `AssertingRandom`) *extend* `java.util.Random`: they override some of its
//! methods and inherit the rest, and every inherited method calls the
//! overridden ones virtually. The port mirrors that with one trait,
//! [`Random`], whose provided methods are `java.util.Random`'s (and the
//! `RandomGenerator` defaults it inherits) and whose implementors override
//! what the Java subclass overrides. [`JdkRandom`] is `java.util.Random`
//! itself, the 48-bit linear congruential generator.
//!
//! Every algorithm is the one JDK 25's javadoc specifies or, where the javadoc
//! only states the contract, the one JDK 25 is observed to run: each method is
//! pinned against a JDK 25 oracle (20 seeds × 10 000 draws per method and
//! parameter, `tests/fixtures/random_draws.tsv`). No JDK source is copied.
//!
//! `nextExponential()` and `nextGaussian(mean, stddev)` are McFarland's
//! modified ziggurat with Walker's alias method, as JDK 25 runs it
//! (`RandomSupport.computeNextExponential`, `computeNextGaussian`,
//! `RandomSupport.java:1105-1402`), written from that description. Their
//! numeric tables (`RandomSupport.DoubleZigguratTables`,
//! `RandomSupport.java:2524`) cannot be recomputed bit-exactly — the 256-slot
//! alias maps depend on the slot-pairing order of the generator program the
//! JDK does not ship — so they are the values the oracle reads from a JDK 25
//! JVM by reflection (`data/ziggurat_tables.rs`, OpenJDK data under GPLv2
//! with the Classpath Exception; see the crate's `NOTICE`).
//!
//! **Divergence.** The ziggurat's rejection test compares a point against
//! `Math.exp` (`RandomSupport.java:1209`, `1332`, `1375`, `1395`), which
//! HotSpot replaces with its own intrinsic and whose javadoc allows any
//! result within 1 ulp. The port calls the platform `f64::exp`. The two
//! agree on every draw of the oracle grid (3 generators × 20 seeds × 10 000
//! draws per method and parameter, `tests/fixtures/ziggurat_draws.tsv`); a
//! draw could differ only if a point fell within 1 ulp of the curve where the
//! two `exp` implementations round differently. HotSpot's intrinsic is not a
//! specified function that Rust can call, so no faithful option exists.
//!
//! Methods take `&self`: like `java.util.Random`, a generator is shared by
//! reference (`RandomizedContext.getRandom()` hands out the same instance on
//! every call), so implementors keep their state in atomics.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use super::exception::illegal_argument;

/// `RandomSupport.DoubleZigguratTables` (JDK 25), dumped from the JVM.
mod ziggurat {
    include!("data/ziggurat_tables.rs");
}
use super::{math, strict_math};

const MULTIPLIER: i64 = 0x5_DEEC_E66D;
const ADDEND: i64 = 0xB;
const MASK: i64 = (1 << 48) - 1;
/// `1.0 / (1L << 53)`.
const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;
/// `1.0f / (1 << 24)`.
const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;

/// `0x1.0p63`.
const TWO_63: f64 = 9_223_372_036_854_775_808.0;
/// `RandomSupport.MAX_EXPONENTIAL`: the largest value
/// `computeNextExponential` returns, `0x1.0p63 * exponentialX0`.
const MAX_EXPONENTIAL: f64 = TWO_63 * ziggurat::EXPONENTIAL_X0;

/// `RandomSupport.BAD_BOUND`.
const BAD_BOUND: &str = "bound must be positive";
/// `RandomSupport.BAD_FLOATING_BOUND`.
const BAD_FLOATING_BOUND: &str = "bound must be finite and positive";
/// `RandomSupport.BAD_RANGE`.
const BAD_RANGE: &str = "bound must be greater than origin";
/// `RandomSupport.BAD_SIZE`.
const BAD_SIZE: &str = "size must be non-negative";

/// The fields every `java.util.Random` (and so every subclass) carries: the
/// 48-bit seed and the cached second Gaussian of `nextGaussian`.
#[derive(Debug)]
pub struct RandomFields {
    seed: AtomicI64,
    /// `(nextNextGaussian, haveNextNextGaussian)`; the lock is the monitor
    /// of the `synchronized` `nextGaussian` and `setSeed`.
    gaussian: Mutex<(f64, bool)>,
}

impl RandomFields {
    /// The fields `new Random(seed)` initializes: `initialScramble(seed)`.
    pub fn new(seed: i64) -> RandomFields {
        RandomFields {
            seed: AtomicI64::new(initial_scramble(seed)),
            gaussian: Mutex::new((0.0, false)),
        }
    }

    /// `java.util.Random.next(bits)`: advances the seed atomically (a
    /// compare-and-set loop, as Java's) and returns its top `bits` bits.
    fn next(&self, bits: i32) -> i32 {
        let mut old = self.seed.load(Ordering::Relaxed);
        loop {
            let next = old.wrapping_mul(MULTIPLIER).wrapping_add(ADDEND) & MASK;
            // Relaxed suffices: the seed is the only state the CAS publishes,
            // as with Java's AtomicLong.compareAndSet on this field alone.
            match self
                .seed
                .compare_exchange_weak(old, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => return ((next as u64) >> (48 - bits)) as i32,
                Err(current) => old = current,
            }
        }
    }

    /// `java.util.Random.setSeed(seed)`.
    fn set_seed(&self, seed: i64) {
        let mut g = self.gaussian.lock().unwrap_or_else(|e| e.into_inner());
        self.seed.store(initial_scramble(seed), Ordering::Relaxed);
        g.1 = false;
    }
}

/// `Random.initialScramble(seed)`.
fn initial_scramble(seed: i64) -> i64 {
    (seed ^ MULTIPLIER) & MASK
}

/// `java.util.Random`: its overridable methods, with JDK 25's
/// implementations as the provided ones.
pub trait Random: Send + Sync {
    /// The `java.util.Random` fields of this instance (Java's `super` state).
    fn fields(&self) -> &RandomFields;

    /// `protected int next(int bits)`: the top `bits` (1..=32) bits of the
    /// next 48-bit LCG step.
    fn next(&self, bits: i32) -> i32 {
        self.fields().next(bits)
    }

    /// `setSeed(long seed)`.
    fn set_seed(&self, seed: i64) {
        self.fields().set_seed(seed)
    }

    /// `nextBytes(byte[] bytes)`: each `nextInt()` fills four bytes, low
    /// byte first. Java `byte`s are the same bits as these `u8`s.
    fn next_bytes(&self, bytes: &mut [u8]) {
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let mut rnd = self.next_int();
            let mut n = (len - i).min(4);
            while n > 0 {
                bytes[i] = rnd as u8;
                i += 1;
                rnd >>= 8;
                n -= 1;
            }
        }
    }

    /// `nextInt()`: `next(32)`.
    fn next_int(&self) -> i32 {
        self.next(32)
    }

    /// `nextInt(int bound)`: uniform in `[0, bound)`; a power of two takes
    /// the high bits of `next(31)`, any other bound rejects the values that
    /// would bias the remainder.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be positive`) when
    /// `bound <= 0`.
    fn next_int_bound(&self, bound: i32) -> i32 {
        if bound <= 0 {
            illegal_argument(BAD_BOUND);
        }
        let mut r = self.next(31);
        let m = bound - 1;
        if bound & m == 0 {
            ((i64::from(bound) * i64::from(r)) >> 31) as i32
        } else {
            let mut u = r;
            loop {
                r = u % bound;
                if u.wrapping_sub(r).wrapping_add(m) >= 0 {
                    return r;
                }
                u = self.next(31);
            }
        }
    }

    /// `nextInt(int origin, int bound)` (`RandomGenerator` default): uniform
    /// in `[origin, bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`) when
    /// `origin >= bound`.
    fn next_int_range(&self, origin: i32, bound: i32) -> i32 {
        if origin >= bound {
            illegal_argument(BAD_RANGE);
        }
        bounded_next_int(self, origin, bound)
    }

    /// `nextLong()`: `((long) next(32) << 32) + next(32)`.
    fn next_long(&self) -> i64 {
        (i64::from(self.next(32)) << 32).wrapping_add(i64::from(self.next(32)))
    }

    /// `nextLong(long bound)` (`RandomGenerator` default).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be positive`) when
    /// `bound <= 0`.
    fn next_long_bound(&self, bound: i64) -> i64 {
        if bound <= 0 {
            illegal_argument(BAD_BOUND);
        }
        bounded_next_long(self, bound)
    }

    /// `nextLong(long origin, long bound)` (`RandomGenerator` default).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`) when
    /// `origin >= bound`.
    fn next_long_range(&self, origin: i64, bound: i64) -> i64 {
        if origin >= bound {
            illegal_argument(BAD_RANGE);
        }
        bounded_next_long_range(self, origin, bound)
    }

    /// `nextBoolean()`: `next(1) != 0`.
    fn next_boolean(&self) -> bool {
        self.next(1) != 0
    }

    /// `nextFloat()`: `next(24) / (float) (1 << 24)`.
    fn next_float(&self) -> f32 {
        self.next(24) as f32 * FLOAT_UNIT
    }

    /// `nextFloat(float bound)` (`RandomGenerator` default).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be finite and positive`)
    /// unless `0 < bound < +inf`.
    fn next_float_bound(&self, bound: f32) -> f32 {
        if !(bound > 0.0 && bound < f32::INFINITY) {
            illegal_argument(BAD_FLOATING_BOUND);
        }
        let mut r = self.next_float() * bound;
        if r >= bound {
            r = math::next_down_f32(bound);
        }
        r
    }

    /// `nextFloat(float origin, float bound)` (`RandomGenerator` default).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`)
    /// unless both are finite and `origin < bound`.
    fn next_float_range(&self, origin: f32, bound: f32) -> f32 {
        if !(origin.is_finite() && bound.is_finite() && origin < bound) {
            illegal_argument(BAD_RANGE);
        }
        let r = self.next_float();
        bounded_float(r, origin, bound)
    }

    /// `nextDouble()`: `(((long) next(26) << 27) + next(27)) * 0x1.0p-53`.
    fn next_double(&self) -> f64 {
        ((i64::from(self.next(26)) << 27) + i64::from(self.next(27))) as f64 * DOUBLE_UNIT
    }

    /// `nextDouble(double bound)` (`RandomGenerator` default).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be finite and positive`)
    /// unless `0 < bound < +inf`.
    fn next_double_bound(&self, bound: f64) -> f64 {
        if !(bound > 0.0 && bound < f64::INFINITY) {
            illegal_argument(BAD_FLOATING_BOUND);
        }
        let mut r = self.next_double() * bound;
        if r >= bound {
            r = math::next_down(bound);
        }
        r
    }

    /// `nextDouble(double origin, double bound)` (`RandomGenerator`
    /// default).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`)
    /// unless both are finite and `origin < bound`.
    fn next_double_range(&self, origin: f64, bound: f64) -> f64 {
        if !(origin.is_finite() && bound.is_finite() && origin < bound) {
            illegal_argument(BAD_RANGE);
        }
        let r = self.next_double();
        bounded_double(r, origin, bound)
    }

    /// `nextGaussian()`: `java.util.Random`'s polar method (Box, Muller and
    /// Marsaglia), which draws pairs and caches the second value; it uses
    /// `nextDouble()`, `StrictMath.sqrt` and `StrictMath.log`.
    fn next_gaussian(&self) -> f64 {
        let mut g = self
            .fields()
            .gaussian
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if g.1 {
            g.1 = false;
            return g.0;
        }
        loop {
            let v1 = 2.0 * self.next_double() - 1.0;
            let v2 = 2.0 * self.next_double() - 1.0;
            let s = v1 * v1 + v2 * v2;
            if s < 1.0 && s != 0.0 {
                let multiplier = strict_math::sqrt(-2.0 * strict_math::log(s) / s);
                *g = (v2 * multiplier, true);
                return v1 * multiplier;
            }
        }
    }

    /// `nextExponential()` (`RandomGenerator` default): a nonnegative value
    /// from the exponential distribution of mean 1, by the modified ziggurat
    /// over `nextLong()`.
    fn next_exponential(&self) -> f64 {
        compute_next_exponential(self)
    }

    /// `nextGaussian(mean, stddev)` (`RandomGenerator` default):
    /// `mean + stddev * g`, with `g` from the standard normal distribution by
    /// the modified ziggurat over `nextLong()` (not `nextGaussian()`).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`"standard deviation must be
    /// non-negative"`) when `stddev < 0`.
    fn next_gaussian_mean_stddev(&self, mean: f64, stddev: f64) -> f64 {
        if stddev < 0.0 {
            illegal_argument("standard deviation must be non-negative");
        }
        mean + stddev * compute_next_gaussian(self)
    }

    /// `isDeprecated()` (`RandomGenerator` default): `false`.
    fn is_deprecated(&self) -> bool {
        false
    }

    /// `ints()`: an unbounded stream of `nextInt()`.
    fn ints(&self) -> Ints<'_, Self>
    where
        Self: Sized,
    {
        Ints::new(self, None, None)
    }

    /// `ints(long streamSize)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`size must be non-negative`).
    fn ints_sized(&self, stream_size: i64) -> Ints<'_, Self>
    where
        Self: Sized,
    {
        check_size(stream_size);
        Ints::new(self, Some(stream_size), None)
    }

    /// `ints(int origin, int bound)`: an unbounded stream in
    /// `[origin, bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`).
    fn ints_range(&self, origin: i32, bound: i32) -> Ints<'_, Self>
    where
        Self: Sized,
    {
        if origin >= bound {
            illegal_argument(BAD_RANGE);
        }
        Ints::new(self, None, Some((origin, bound)))
    }

    /// `ints(long streamSize, int origin, int bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for a negative size or an empty range.
    fn ints_sized_range(&self, stream_size: i64, origin: i32, bound: i32) -> Ints<'_, Self>
    where
        Self: Sized,
    {
        check_size(stream_size);
        if origin >= bound {
            illegal_argument(BAD_RANGE);
        }
        Ints::new(self, Some(stream_size), Some((origin, bound)))
    }

    /// `longs()`.
    fn longs(&self) -> Longs<'_, Self>
    where
        Self: Sized,
    {
        Longs::new(self, None, None)
    }

    /// `longs(long streamSize)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`size must be non-negative`).
    fn longs_sized(&self, stream_size: i64) -> Longs<'_, Self>
    where
        Self: Sized,
    {
        check_size(stream_size);
        Longs::new(self, Some(stream_size), None)
    }

    /// `longs(long origin, long bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`).
    fn longs_range(&self, origin: i64, bound: i64) -> Longs<'_, Self>
    where
        Self: Sized,
    {
        if origin >= bound {
            illegal_argument(BAD_RANGE);
        }
        Longs::new(self, None, Some((origin, bound)))
    }

    /// `longs(long streamSize, long origin, long bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for a negative size or an empty range.
    fn longs_sized_range(&self, stream_size: i64, origin: i64, bound: i64) -> Longs<'_, Self>
    where
        Self: Sized,
    {
        check_size(stream_size);
        if origin >= bound {
            illegal_argument(BAD_RANGE);
        }
        Longs::new(self, Some(stream_size), Some((origin, bound)))
    }

    /// `doubles()`.
    fn doubles(&self) -> Doubles<'_, Self>
    where
        Self: Sized,
    {
        Doubles::new(self, None, None)
    }

    /// `doubles(long streamSize)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`size must be non-negative`).
    fn doubles_sized(&self, stream_size: i64) -> Doubles<'_, Self>
    where
        Self: Sized,
    {
        check_size(stream_size);
        Doubles::new(self, Some(stream_size), None)
    }

    /// `doubles(double origin, double bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`bound must be greater than origin`)
    /// unless both are finite and `origin < bound`.
    fn doubles_range(&self, origin: f64, bound: f64) -> Doubles<'_, Self>
    where
        Self: Sized,
    {
        check_double_range(origin, bound);
        Doubles::new(self, None, Some((origin, bound)))
    }

    /// `doubles(long streamSize, double origin, double bound)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for a negative size or an invalid range.
    fn doubles_sized_range(&self, stream_size: i64, origin: f64, bound: f64) -> Doubles<'_, Self>
    where
        Self: Sized,
    {
        check_size(stream_size);
        check_double_range(origin, bound);
        Doubles::new(self, Some(stream_size), Some((origin, bound)))
    }

    /// `equiDoubles(double left, double right, boolean isLeftIncluded,
    /// boolean isRightIncluded)` (`RandomGenerator` default, JDK 22+): an
    /// unlimited stream of the equidistant doubles `k·δ` in the interval,
    /// where `δ` is the smallest power of two for which every such multiple
    /// is exact.
    ///
    /// As the default implementation specifies: `δ` is the distance from
    /// `left` to its upper neighbour when `left <= -right`, else from `right`
    /// to its lower neighbour; `kl` is the smallest `k` with `k·δ` inside the
    /// interval and `kr` the smallest beyond it; each value is
    /// `(kl + nextLong(kr - kl)) · δ`. A quotient that underflows to zero is
    /// rounded away from zero first so that `floor`/`ceil` see its sign.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`the boundaries must be finite and the
    /// interval must not be empty`).
    fn equi_doubles(
        &self,
        left: f64,
        right: f64,
        is_left_included: bool,
        is_right_included: bool,
    ) -> EquiDoubles<'_, Self>
    where
        Self: Sized,
    {
        let low = if is_left_included {
            left
        } else {
            math::next_up(left)
        };
        let high = if is_right_included {
            right
        } else {
            math::next_down(right)
        };
        if !(f64::NEG_INFINITY < left && right < f64::INFINITY && low <= high) {
            illegal_argument("the boundaries must be finite and the interval must not be empty");
        }
        // A quotient that underflowed to zero keeps the sign of its dividend.
        let away_from_zero = |q: f64, dividend: f64| {
            if q != 0.0 || dividend == 0.0 {
                q
            } else {
                f64::from_bits(1).copysign(dividend)
            }
        };
        let (delta, kl, kr);
        if left <= -right {
            delta = math::next_up(left) - left;
            let rd = away_from_zero(right / delta, right);
            kr = if is_right_included {
                (rd.floor() as i64).wrapping_add(1)
            } else {
                rd.ceil() as i64
            };
            kl = ((left / delta) as i64).wrapping_add(i64::from(!is_left_included));
        } else {
            delta = right - math::next_down(right);
            let ld = away_from_zero(left / delta, left);
            kl = if is_left_included {
                ld.ceil() as i64
            } else {
                (ld.floor() as i64).wrapping_add(1)
            };
            kr = ((right / delta) as i64).wrapping_add(i64::from(is_right_included));
        }
        EquiDoubles {
            rng: self,
            delta,
            kl,
            n: kr.wrapping_sub(kl),
        }
    }
}

/// `nextLong() >>> 1`.
fn next_long_63<R: Random + ?Sized>(rng: &R) -> i64 {
    ((rng.next_long() as u64) >> 1) as i64
}

/// The x (or y) coordinate of a point in ziggurat layer `j`:
/// `t[j] * 2^63 + (t[j - 1] - t[j]) * u`, unfused as Java evaluates it.
fn layer_coordinate(t: &[f64], j: usize, u: i64) -> f64 {
    (t[j] * TWO_63) + ((t[j - 1] - t[j]) * u as f64)
}

/// The same coordinate fused, `Math.fma(t[j - 1] - t[j], u, t[j] * 2^63)`,
/// as the inflection layer of the normal ziggurat computes it.
fn layer_coordinate_fma(t: &[f64], j: usize, u: i64) -> f64 {
    (t[j - 1] - t[j]).mul_add(u as f64, t[j] * TWO_63)
}

/// Walker's alias draw of an overhang (or the tail, `0`) from `nextLong()`.
fn alias_sample<R: Random + ?Sized>(
    rng: &R,
    mask: i32,
    threshold: &[i64; 256],
    map: &[i8; 256],
    sign_correction: i32,
) -> usize {
    let ua = rng.next_long();
    let mut j = (ua as i32) & mask;
    if ua >= threshold[j as usize] {
        j = i32::from(map[j as usize]) & sign_correction;
    }
    j as usize
}

/// `RandomSupport.computeNextExponential(rng)`.
pub(crate) fn compute_next_exponential<R: Random + ?Sized>(rng: &R) -> f64 {
    compute_next_exponential_soft_capped(rng, MAX_EXPONENTIAL)
}

/// `RandomSupport.computeNextExponentialSoftCapped(rng, maxValue)`: an
/// exponential draw whose `min(x, maxValue)` has the exponential
/// distribution, with a number of `nextLong()` calls bounded by `maxValue`.
pub(crate) fn compute_next_exponential_soft_capped<R: Random + ?Sized>(
    rng: &R,
    max_value: f64,
) -> f64 {
    use ziggurat::*;
    let mut u1 = rng.next_long();
    let i = u1 & i64::from(EXPONENTIAL_LAYER_MASK);
    if i < i64::from(EXPONENTIAL_NUMBER_OF_LAYERS) {
        return EXPONENTIAL_X[i as usize] * ((u1 as u64) >> 1) as f64;
    }
    if max_value <= 0.0 {
        return 0.0;
    }
    let max_extra_minus_1 = if max_value >= MAX_EXPONENTIAL {
        i64::MAX
    } else {
        // Java's (long) conversion: toward zero, saturating, NaN to 0.
        (max_value / EXPONENTIAL_X0) as i64
    };
    let mut extra: i64 = 0;
    loop {
        let j = alias_sample(
            rng,
            EXPONENTIAL_ALIAS_MASK,
            &EXPONENTIAL_ALIAS_THRESHOLD,
            &EXPONENTIAL_ALIAS_MAP,
            EXPONENTIAL_SIGN_CORRECTION_MASK,
        );
        if j > 0 {
            // Overhang j; every exponential overhang is convex.
            u1 = ((u1 as u64) >> 1) as i64;
            loop {
                let mut u2 = next_long_63(rng);
                let mut u_diff = u2 - u1;
                if u_diff < 0 {
                    // Upper-right triangle: mirror into the lower-left one.
                    u_diff = -u_diff;
                    u2 = u1;
                    u1 -= u_diff;
                }
                let x = layer_coordinate(&EXPONENTIAL_X, j, u1);
                if u_diff >= EXPONENTIAL_CONVEX_MARGIN {
                    return (extra as f64).mul_add(EXPONENTIAL_X0, x);
                }
                let y = layer_coordinate(&EXPONENTIAL_Y, j, u2);
                if y <= (-x).exp() {
                    return (extra as f64).mul_add(EXPONENTIAL_X0, x);
                }
                u1 = next_long_63(rng);
            }
        }
        if extra == max_extra_minus_1 {
            return max_value;
        }
        // The tail: another draw, shifted by `extra` times X0.
        extra += 1;
        u1 = rng.next_long();
        let i = u1 & i64::from(EXPONENTIAL_LAYER_MASK);
        if i < i64::from(EXPONENTIAL_NUMBER_OF_LAYERS) {
            return (extra as f64).mul_add(
                EXPONENTIAL_X0,
                EXPONENTIAL_X[i as usize] * ((u1 as u64) >> 1) as f64,
            );
        }
    }
}

/// `RandomSupport.computeNextGaussian(rng)`: a standard normal draw.
pub(crate) fn compute_next_gaussian<R: Random + ?Sized>(rng: &R) -> f64 {
    use ziggurat::*;
    let mut u1 = rng.next_long();
    let i = u1 & i64::from(NORMAL_LAYER_MASK);
    if i < i64::from(NORMAL_NUMBER_OF_LAYERS) {
        // The sign bit of u1 is the sign of the result.
        return NORMAL_X[i as usize] * u1 as f64;
    }
    let sign_bit = if u1 >= 0 { 1.0 } else { -1.0 };
    u1 = (((u1 as u64) << 1) >> 1) as i64;
    let j = alias_sample(
        rng,
        NORMAL_ALIAS_MASK,
        &NORMAL_ALIAS_THRESHOLD,
        &NORMAL_ALIAS_MAP,
        NORMAL_SIGN_CORRECTION_MASK,
    );
    let inflection = NORMAL_INFLECTION_INDEX as usize;
    let curve = |x: f64| (-0.5 * x * x).exp();
    let x;
    if j > inflection {
        // Concave overhang: accept the lower-left triangle outright.
        loop {
            let u2 = next_long_63(rng);
            let cx = layer_coordinate(&NORMAL_X, j, u1);
            let u_diff = u2 - u1;
            let accepted = u_diff >= 0
                || (u_diff > -NORMAL_CONCAVE_MARGIN
                    && layer_coordinate(&NORMAL_Y, j, u2) <= curve(cx));
            if accepted {
                x = cx;
                break;
            }
            u1 = next_long_63(rng);
        }
    } else if j == 0 {
        // The tail (Marsaglia and Tsang): two exponential draws compared.
        let mut t;
        loop {
            t = (1.0 / NORMAL_X0) * compute_next_exponential(rng);
            let limit = 0.5 * t * t;
            if compute_next_exponential_soft_capped(rng, limit) >= limit {
                break;
            }
        }
        x = t + NORMAL_X0;
    } else if j < inflection {
        // Convex overhang: mirror the upper-right triangle.
        loop {
            let mut u2 = next_long_63(rng);
            let mut u_diff = u2 - u1;
            if u_diff < 0 {
                u_diff = -u_diff;
                u2 = u1;
                u1 -= u_diff;
            }
            let cx = layer_coordinate(&NORMAL_X, j, u1);
            if u_diff >= NORMAL_CONVEX_MARGIN || layer_coordinate(&NORMAL_Y, j, u2) <= curve(cx) {
                x = cx;
                break;
            }
            u1 = next_long_63(rng);
        }
    } else {
        // The overhang holding the inflection point: both margins apply.
        loop {
            let u2 = next_long_63(rng);
            let cx = layer_coordinate_fma(&NORMAL_X, j, u1);
            let u_diff = u2 - u1;
            let accepted = u_diff >= NORMAL_CONVEX_MARGIN
                || (u_diff > -NORMAL_CONCAVE_MARGIN
                    && layer_coordinate_fma(&NORMAL_Y, j, u2) <= curve(cx));
            if accepted {
                x = cx;
                break;
            }
            u1 = next_long_63(rng);
        }
    }
    sign_bit * x
}

fn check_size(stream_size: i64) {
    if stream_size < 0 {
        illegal_argument(BAD_SIZE);
    }
}

fn check_double_range(origin: f64, bound: f64) {
    if !(origin.is_finite() && bound.is_finite() && origin < bound) {
        illegal_argument(BAD_RANGE);
    }
}

/// A value in `[origin, bound)` from one `nextInt()` and, for a range that is
/// not a power of two, the rejection of biased draws.
pub(crate) fn bounded_next_int<R: Random + ?Sized>(rng: &R, origin: i32, bound: i32) -> i32 {
    let mut r = rng.next_int();
    if origin < bound {
        let n = bound.wrapping_sub(origin);
        let m = n.wrapping_sub(1);
        if n & m == 0 {
            r = (r & m).wrapping_add(origin);
        } else if n > 0 {
            let mut u = ((r as u32) >> 1) as i32;
            loop {
                r = u % n;
                if u.wrapping_add(m).wrapping_sub(r) >= 0 {
                    break;
                }
                u = ((rng.next_int() as u32) >> 1) as i32;
            }
            r = r.wrapping_add(origin);
        } else {
            while r < origin || r >= bound {
                r = rng.next_int();
            }
        }
    }
    r
}

/// A value in `[0, bound)` from `nextLong()`.
pub(crate) fn bounded_next_long<R: Random + ?Sized>(rng: &R, bound: i64) -> i64 {
    let m = bound.wrapping_sub(1);
    let mut r = rng.next_long();
    if bound & m == 0 {
        r &= m;
    } else {
        let mut u = ((r as u64) >> 1) as i64;
        loop {
            r = u % bound;
            if u.wrapping_add(m).wrapping_sub(r) >= 0 {
                break;
            }
            u = ((rng.next_long() as u64) >> 1) as i64;
        }
    }
    r
}

/// A value in `[origin, bound)` from `nextLong()`.
pub(crate) fn bounded_next_long_range<R: Random + ?Sized>(rng: &R, origin: i64, bound: i64) -> i64 {
    let mut r = rng.next_long();
    if origin < bound {
        let n = bound.wrapping_sub(origin);
        let m = n.wrapping_sub(1);
        if n & m == 0 {
            r = (r & m).wrapping_add(origin);
        } else if n > 0 {
            let mut u = ((r as u64) >> 1) as i64;
            loop {
                r = u % n;
                if u.wrapping_add(m).wrapping_sub(r) >= 0 {
                    break;
                }
                u = ((rng.next_long() as u64) >> 1) as i64;
            }
            r = r.wrapping_add(origin);
        } else {
            while r < origin || r >= bound {
                r = rng.next_long();
            }
        }
    }
    r
}

/// Scales `r` in `[0, 1)` to `[origin, bound)`; a range too wide for `float`
/// is scaled through its halves.
fn bounded_float(mut r: f32, origin: f32, bound: f32) -> f32 {
    if origin < bound {
        if bound - origin < f32::INFINITY {
            r = r * (bound - origin) + origin;
        } else {
            let half_origin = 0.5 * origin;
            r = (r * (0.5 * bound - half_origin) + half_origin) * 2.0;
        }
        if r >= bound {
            r = math::next_down_f32(bound);
        }
    }
    r
}

/// Scales `r` in `[0, 1)` to `[origin, bound)`; a range too wide for
/// `double` is scaled through its halves.
fn bounded_double(mut r: f64, origin: f64, bound: f64) -> f64 {
    if origin < bound {
        if bound - origin < f64::INFINITY {
            r = r * (bound - origin) + origin;
        } else {
            let half_origin = 0.5 * origin;
            r = (r * (0.5 * bound - half_origin) + half_origin) * 2.0;
        }
        if r >= bound {
            r = math::next_down(bound);
        }
    }
    r
}

macro_rules! stream {
    ($(#[$doc:meta])* $name:ident, $item:ty, $next:expr, $bounded:expr) => {
        $(#[$doc])*
        pub struct $name<'a, R: Random + ?Sized> {
            rng: &'a R,
            remaining: Option<i64>,
            range: Option<($item, $item)>,
        }

        impl<'a, R: Random + ?Sized> $name<'a, R> {
            fn new(rng: &'a R, remaining: Option<i64>, range: Option<($item, $item)>) -> Self {
                Self { rng, remaining, range }
            }
        }

        impl<R: Random + ?Sized> Iterator for $name<'_, R> {
            type Item = $item;

            fn next(&mut self) -> Option<$item> {
                if let Some(remaining) = self.remaining.as_mut() {
                    if *remaining == 0 {
                        return None;
                    }
                    *remaining -= 1;
                }
                let next: fn(&R) -> $item = $next;
                let bounded: fn(&R, $item, $item) -> $item = $bounded;
                Some(match self.range {
                    None => next(self.rng),
                    Some((origin, bound)) => bounded(self.rng, origin, bound),
                })
            }
        }
    };
}

stream!(
    /// The `IntStream` of `ints(...)`, drawn lazily and in order.
    Ints, i32, |r| r.next_int(), |r, o, b| bounded_next_int(r, o, b)
);
stream!(
    /// The `LongStream` of `longs(...)`, drawn lazily and in order.
    Longs, i64, |r| r.next_long(), |r, o, b| bounded_next_long_range(r, o, b)
);
stream!(
    /// The `DoubleStream` of `doubles(...)`, drawn lazily and in order.
    Doubles, f64, |r| r.next_double(), |r, o, b| bounded_double(r.next_double(), o, b)
);

/// The `DoubleStream` of `equiDoubles(...)`, drawn lazily and in order:
/// each element is `(kl + nextLong(n)) · δ`.
pub struct EquiDoubles<'a, R: Random + ?Sized> {
    rng: &'a R,
    delta: f64,
    kl: i64,
    n: i64,
}

impl<R: Random + ?Sized> Iterator for EquiDoubles<'_, R> {
    type Item = f64;

    fn next(&mut self) -> Option<f64> {
        let k = self.kl.wrapping_add(self.rng.next_long_bound(self.n));
        Some(k as f64 * self.delta)
    }
}

/// `java.util.Random` itself: the 48-bit linear congruential generator
/// (`new Random(seed)`).
#[derive(Debug)]
pub struct JdkRandom {
    fields: RandomFields,
}

impl JdkRandom {
    /// `new Random(seed)`.
    pub fn new(seed: i64) -> JdkRandom {
        JdkRandom {
            fields: RandomFields::new(seed),
        }
    }

    /// `new Random()`: a seed that differs between calls.
    ///
    /// **Divergence.** Java mixes a static "seed uniquifier" with
    /// `System.nanoTime()`; the port mixes a process-wide counter with the
    /// system clock. Neither is reproducible, which is the contract.
    pub fn new_unseeded() -> JdkRandom {
        static UNIQUIFIER: AtomicI64 = AtomicI64::new(0x0123_4567_89AB_CDEF);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);
        let u = UNIQUIFIER
            .fetch_add(0x9E37_79B9_7F4A_7C15_u64 as i64, Ordering::Relaxed)
            .wrapping_mul(0x2545_F491_4F6C_DD1D);
        JdkRandom::new(u ^ nanos)
    }
}

impl Random for JdkRandom {
    fn fields(&self) -> &RandomFields {
        &self.fields
    }
}

macro_rules! delegate_random {
    ($($ty:ty),*) => {$(
        impl<R: Random + ?Sized> Random for $ty {
    fn fields(&self) -> &RandomFields {
                (**self).fields()
            }
            fn next(&self, bits: i32) -> i32 {
                (**self).next(bits)
            }
            fn set_seed(&self, seed: i64) {
                (**self).set_seed(seed)
            }
            fn next_bytes(&self, bytes: &mut [u8]) {
                (**self).next_bytes(bytes)
            }
            fn next_int(&self) -> i32 {
                (**self).next_int()
            }
            fn next_int_bound(&self, bound: i32) -> i32 {
                (**self).next_int_bound(bound)
            }
            fn next_int_range(&self, origin: i32, bound: i32) -> i32 {
                (**self).next_int_range(origin, bound)
            }
            fn next_long(&self) -> i64 {
                (**self).next_long()
            }
            fn next_long_bound(&self, bound: i64) -> i64 {
                (**self).next_long_bound(bound)
            }
            fn next_long_range(&self, origin: i64, bound: i64) -> i64 {
                (**self).next_long_range(origin, bound)
            }
            fn next_boolean(&self) -> bool {
                (**self).next_boolean()
            }
            fn next_float(&self) -> f32 {
                (**self).next_float()
            }
            fn next_float_bound(&self, bound: f32) -> f32 {
                (**self).next_float_bound(bound)
            }
            fn next_float_range(&self, origin: f32, bound: f32) -> f32 {
                (**self).next_float_range(origin, bound)
            }
            fn next_double(&self) -> f64 {
                (**self).next_double()
            }
            fn next_double_bound(&self, bound: f64) -> f64 {
                (**self).next_double_bound(bound)
            }
            fn next_double_range(&self, origin: f64, bound: f64) -> f64 {
                (**self).next_double_range(origin, bound)
            }
            fn next_gaussian(&self) -> f64 {
                (**self).next_gaussian()
            }
            fn next_exponential(&self) -> f64 {
                (**self).next_exponential()
            }
            fn next_gaussian_mean_stddev(&self, mean: f64, stddev: f64) -> f64 {
                (**self).next_gaussian_mean_stddev(mean, stddev)
            }
            fn is_deprecated(&self) -> bool {
                (**self).is_deprecated()
            }
        }
    )*};
}

// A shared or borrowed generator is a generator: every call goes to the
// instance it points at, so its overridden methods stay in force (and the
// stream methods, which need a sized receiver, work on `Arc<dyn Random>` and
// `&dyn Random`).
delegate_random!(Arc<R>, &R);

/// `random.ints(streamSize, origin, bound)` on a `Random` trait object (the
/// `Random` that `LuceneTestCase.random()` returns): the same stream as
/// [`Random::ints_sized_range`], which needs a sized receiver.
///
/// # Panics
///
/// `IllegalArgumentException` for a negative size or an empty range.
pub fn ints_sized_range_dyn<'a>(
    rng: &'a (dyn Random + 'a),
    stream_size: i64,
    origin: i32,
    bound: i32,
) -> Ints<'a, dyn Random + 'a> {
    check_size(stream_size);
    if origin >= bound {
        illegal_argument(BAD_RANGE);
    }
    Ints::new(rng, Some(stream_size), Some((origin, bound)))
}
