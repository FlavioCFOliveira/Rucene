//! Port of `com.carrotsearch.randomizedtesting.Xoroshiro128PlusRandom`
//! (randomizedtesting 2.8.1, `Xoroshiro128PlusRandom.java`).

use std::sync::atomic::{AtomicI64, Ordering};

use crate::asserting_random::AssertingRandom;
use crate::jdk::random::{Random, RandomFields};
use crate::murmur_hash3::MurmurHash3;

/// `0x1.0p-53`.
const DOUBLE_UNIT: f64 = 1.0 / (1u64 << 53) as f64;
/// `0x1.0p-24f`.
const FLOAT_UNIT: f32 = 1.0 / (1u32 << 24) as f32;

/// The xoroshiro128+ generator, randomizedtesting's default
/// (`RandomSupplier.DEFAULT`): `Xoroshiro128PlusRandom extends
/// java.util.Random`.
///
/// It overrides `next`, `nextBoolean`, `nextBytes`, `nextDouble`,
/// `nextFloat`, `nextInt()`, `nextLong` and `setSeed`, and inherits the rest
/// of `java.util.Random` (`nextInt(int)`, `nextGaussian`, the bounded and
/// stream methods), which call the overridden ones.
///
/// "Not synchronized (anywhere)": the state is two plain `long`s in Java,
/// two relaxed atomics here (a racing caller reads torn but defined state,
/// as in Java, never undefined behaviour).
#[derive(Debug)]
pub struct Xoroshiro128PlusRandom {
    fields: RandomFields,
    s0: AtomicI64,
    s1: AtomicI64,
}

impl Xoroshiro128PlusRandom {
    /// `new Xoroshiro128PlusRandom(seed)`: the state is the MurmurHash3
    /// finalizer of `seed` and of that hash; the all-zero state, which
    /// xoroshiro cannot leave, is replaced by the hashes of `0xdeadbeef`.
    pub fn new(seed: i64) -> Xoroshiro128PlusRandom {
        let mut s0 = MurmurHash3::hash_long(seed);
        let mut s1 = MurmurHash3::hash_long(s0);
        if s0 == 0 && s1 == 0 {
            s0 = MurmurHash3::hash_long(0xdead_beef);
            s1 = MurmurHash3::hash_long(s0);
        }
        Xoroshiro128PlusRandom {
            // `super(0)`.
            fields: RandomFields::new(0),
            s0: AtomicI64::new(s0),
            s1: AtomicI64::new(s1),
        }
    }
}

impl Random for Xoroshiro128PlusRandom {
    fn fields(&self) -> &RandomFields {
        &self.fields
    }

    /// `next(bits)`: the top `bits` bits of the low 32 bits of `nextLong()`.
    fn next(&self, bits: i32) -> i32 {
        // Java's `>>>` masks the shift distance to 5 bits.
        ((self.next_long() as i32 as u32).wrapping_shr((32 - bits) as u32)) as i32
    }

    /// `setSeed(seed)`: always refused once constructed.
    ///
    /// Java returns silently only for the call `java.util.Random`'s
    /// constructor makes before the state is set (`s0 == 0 && s1 == 0`); the
    /// Rust constructor makes no such call, so the check never passes after
    /// construction, as in Java.
    ///
    /// # Panics
    ///
    /// `RuntimeException` with `AssertingRandom.noSetSeed()`'s message.
    fn set_seed(&self, _seed: i64) {
        if self.s0.load(Ordering::Relaxed) == 0 && self.s1.load(Ordering::Relaxed) == 0 {
            return;
        }
        AssertingRandom::no_set_seed()
    }

    /// `nextBoolean()`: `nextLong() >= 0`.
    fn next_boolean(&self) -> bool {
        self.next_long() >= 0
    }

    /// `nextBytes(bytes)`: each `nextInt()`, sign-extended to `long`, fills
    /// up to eight bytes, low byte first (so bytes 4..8 of each group are the
    /// sign of the `int`).
    fn next_bytes(&self, bytes: &mut [u8]) {
        let len = bytes.len();
        let mut i = 0;
        while i < len {
            let mut rnd = i64::from(self.next_int()) as u64;
            let mut n = (len - i).min(8);
            while n > 0 {
                bytes[i] = rnd as u8;
                i += 1;
                rnd >>= 8;
                n -= 1;
            }
        }
    }

    /// `nextDouble()`: `(nextLong() >>> 11) * 0x1.0p-53`.
    fn next_double(&self) -> f64 {
        ((self.next_long() as u64) >> 11) as f64 * DOUBLE_UNIT
    }

    /// `nextFloat()`: `(nextInt() >>> 8) * 0x1.0p-24f`.
    fn next_float(&self) -> f32 {
        ((self.next_int() as u32) >> 8) as f32 * FLOAT_UNIT
    }

    /// `nextInt()`: `(int) nextLong()`.
    fn next_int(&self) -> i32 {
        self.next_long() as i32
    }

    /// `nextLong()`: one xoroshiro128+ step (rotations 55 and 36, shift 14).
    fn next_long(&self) -> i64 {
        // Relaxed: Java's fields are plain and unsynchronized.
        let s0 = self.s0.load(Ordering::Relaxed);
        let mut s1 = self.s1.load(Ordering::Relaxed);
        let result = s0.wrapping_add(s1);
        s1 ^= s0;
        self.s0
            .store(s0.rotate_left(55) ^ s1 ^ (s1 << 14), Ordering::Relaxed);
        self.s1.store(s1.rotate_left(36), Ordering::Relaxed);
        result
    }
}
