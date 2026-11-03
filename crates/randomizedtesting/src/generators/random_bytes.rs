//! Port of `com.carrotsearch.randomizedtesting.generators.RandomBytes` (randomizedtesting 2.8.1).

use super::{array_length, RandomNumbers};
use crate::jdk::random::Random;

/// Random byte sequences: `RandomBytes`. Java `byte`s are the same bits as
/// these `u8`s.
pub struct RandomBytes;

impl RandomBytes {
    /// `randomBytesOfLength(r, length)`.
    ///
    /// # Panics
    ///
    /// `NegativeArraySizeException` for a negative length.
    pub fn random_bytes_of_length<R: Random + ?Sized>(r: &R, length: i32) -> Vec<u8> {
        RandomBytes::random_bytes_of_length_between(r, length, length)
    }

    /// `randomBytesOfLengthBetween(r, minLength, maxLength)`: each byte is
    /// `(byte) r.nextInt()`.
    ///
    /// # Panics
    ///
    /// `NegativeArraySizeException` for a negative drawn length.
    pub fn random_bytes_of_length_between<R: Random + ?Sized>(
        r: &R,
        min_length: i32,
        max_length: i32,
    ) -> Vec<u8> {
        let len = array_length(RandomNumbers::random_int_between(r, min_length, max_length));
        (0..len).map(|_| r.next_int() as u8).collect()
    }
}
