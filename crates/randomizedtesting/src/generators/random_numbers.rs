//! Port of `com.carrotsearch.randomizedtesting.generators.RandomNumbers` (randomizedtesting 2.8.1).

use crate::jdk::exception::{self, illegal_argument, java_assert};
use crate::jdk::random::Random;

/// Random numbers within a range or a type's domain: `RandomNumbers`.
pub struct RandomNumbers;

impl RandomNumbers {
    /// `randomIntBetween(r, min, max)`: uniform in `[min, max]`.
    ///
    /// # Panics
    ///
    /// `AssertionError` (`max must be >= min: <min>, <max>`) when
    /// `max < min` and assertions are on.
    pub fn random_int_between<R: Random + ?Sized>(r: &R, min: i32, max: i32) -> i32 {
        java_assert(max >= min, || format!("max must be >= min: {min}, {max}"));
        let range = i64::from(max) - i64::from(min);
        if range < i64::from(i32::MAX) {
            min.wrapping_add(r.next_int_bound(1i32.wrapping_add(range as i32)))
        } else {
            RandomNumbers::to_int_exact(
                i64::from(min).wrapping_add(RandomNumbers::next_long(r, 1 + range)),
            )
        }
    }

    /// `randomLongBetween(r, min, max)`: uniform in `[min, max]`.
    ///
    /// # Panics
    ///
    /// `AssertionError` (`max must be >= min: <min>, <max>`) when
    /// `max < min` and assertions are on.
    pub fn random_long_between<R: Random + ?Sized>(r: &R, min: i64, max: i64) -> i64 {
        java_assert(max >= min, || format!("max must be >= min: {min}, {max}"));
        let mut range = max.wrapping_sub(min);
        if range < 0 {
            range = range.wrapping_sub(i64::MAX);
            if range == i64::MIN {
                // Full spectrum.
                r.next_long()
            } else {
                let first = r.next_long() & i64::MAX;
                let second = if range == i64::MAX {
                    r.next_long() & i64::MAX
                } else {
                    RandomNumbers::next_long(r, range + 1)
                };
                min.wrapping_add(first).wrapping_add(second)
            }
        } else {
            let second = if range == i64::MAX {
                r.next_long() & i64::MAX
            } else {
                RandomNumbers::next_long(r, range + 1)
            };
            min.wrapping_add(second)
        }
    }

    /// `nextLong(rnd, n)`: like `Random.nextInt(int)`, a `long` in
    /// `[0, n)`.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`n <= 0: <n>`) when `n <= 0`.
    pub fn next_long<R: Random + ?Sized>(rnd: &R, n: i64) -> i64 {
        if n <= 0 {
            illegal_argument(format!("n <= 0: {n}"));
        }
        let mut value = rnd.next_long();
        let range = n - 1;
        if n & range == 0 {
            value &= range;
        } else {
            let mut u = ((value as u64) >> 1) as i64;
            loop {
                value = u % n;
                if u.wrapping_add(range).wrapping_sub(value) >= 0 {
                    break;
                }
                u = ((rnd.next_long() as u64) >> 1) as i64;
            }
        }
        value
    }

    /// `toIntExact(long)`.
    fn to_int_exact(value: i64) -> i32 {
        if value > i64::from(i32::MAX) {
            exception::throw(
                exception::ARITHMETIC_EXCEPTION,
                format!("Overflow: {value}"),
            )
        }
        value as i32
    }
}
