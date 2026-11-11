//! Port of `com.carrotsearch.randomizedtesting.MixWithSuiteName`
//! (randomizedtesting 2.8.1, `MixWithSuiteName.java`).

use crate::jdk::lang::string_hash_code;
use crate::seed_decorator::SeedDecorator;

/// A [`SeedDecorator`] that XORs the seed with a hash of the suite's class
/// name: `MixWithSuiteName`.
#[derive(Debug, Clone, Copy, Default)]
pub struct MixWithSuiteName {
    xor_hash: i64,
}

impl MixWithSuiteName {
    /// `new MixWithSuiteName()`.
    pub fn new() -> MixWithSuiteName {
        MixWithSuiteName::default()
    }

    /// `fmix64(long)`: the final mix of MurmurHash3.
    fn fmix64(k: i64) -> i64 {
        let mut k = k as u64;
        k ^= k >> 33;
        k = k.wrapping_mul(0xff51_afd7_ed55_8ccd);
        k ^= k >> 33;
        k = k.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
        k ^= k >> 33;
        k as i64
    }
}

impl SeedDecorator for MixWithSuiteName {
    /// `xorHash = fmix64(suiteClass.getName().hashCode())`, the `int` hash
    /// sign-extended to `long`.
    fn initialize(&mut self, suite_class_name: &str) {
        self.xor_hash = MixWithSuiteName::fmix64(i64::from(string_hash_code(suite_class_name)));
    }

    fn decorate(&self, seed: i64) -> i64 {
        seed ^ self.xor_hash
    }
}
