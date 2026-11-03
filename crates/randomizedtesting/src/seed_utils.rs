//! Port of `com.carrotsearch.randomizedtesting.SeedUtils`
//! (randomizedtesting 2.8.1, `SeedUtils.java`).

use crate::jdk::exception::illegal_argument;
use crate::randomness::Randomness;

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Utilities for parsing and formatting random seeds: `SeedUtils`.
pub struct SeedUtils;

impl SeedUtils {
    /// `parseSeed(String)`: a single hexadecimal seed, already stripped of
    /// surrounding characters; digits past the sixteenth shift the earlier
    /// ones out, as Java's `long` shifts do.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Expected hexadecimal seed: <seed>`) on a
    /// non-hexadecimal character.
    pub fn parse_seed(seed: &str) -> i64 {
        let mut result: i64 = 0;
        for unit in seed.encode_utf16() {
            // `Character.toLowerCase(char)`: no non-ASCII char lowercases to
            // an ASCII hex digit, so ASCII lowercasing decides the same set.
            let chr = if unit < 0x80 {
                (unit as u8).to_ascii_lowercase()
            } else {
                illegal_argument(format!("Expected hexadecimal seed: {seed}"))
            };
            result = result.wrapping_shl(4);
            match chr {
                b'0'..=b'9' => result |= i64::from(chr - b'0'),
                b'a'..=b'f' => result |= i64::from(chr - b'a' + 10),
                _ => illegal_argument(format!("Expected hexadecimal seed: {seed}")),
            }
        }
        result
    }

    /// `formatSeed(long)`: upper-case hexadecimal of the unsigned value,
    /// without leading zeros.
    pub fn format_seed(seed: i64) -> String {
        let mut seed = seed as u64;
        let mut b = Vec::new();
        loop {
            b.push(HEX[(seed & 0xF) as usize]);
            seed >>= 4;
            if seed == 0 {
                break;
            }
        }
        b.reverse();
        String::from_utf8(b).expect("ASCII")
    }

    /// `parseSeedChain(String)`: a chain formatted by
    /// [`format_seed_chain`](Self::format_seed_chain), brackets ignored.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Not a valid seed chain: <chain>`, the
    /// chain without brackets) unless what remains is one or more of
    /// `[0-9A-Fa-f:]`.
    pub fn parse_seed_chain(chain: &str) -> Vec<i64> {
        // chain.replaceAll("[\\[\\]]", "")
        let chain: String = chain.chars().filter(|c| *c != '[' && *c != ']').collect();
        // chain.matches("[0-9A-Fa-f\\:]+")
        if chain.is_empty() || !chain.chars().all(|c| c.is_ascii_hexdigit() || c == ':') {
            illegal_argument(format!("Not a valid seed chain: {chain}"));
        }
        // chain.split("[\\:]"), which drops trailing empty strings.
        let mut splits: Vec<&str> = chain.split(':').collect();
        while splits.last().is_some_and(|s| s.is_empty()) {
            splits.pop();
        }
        splits.into_iter().map(SeedUtils::parse_seed).collect()
    }

    /// `formatSeedChain(Randomness...)`: `[S1:S2:...]`, something a user can
    /// type in to repeat the execution.
    pub fn format_seed_chain(randomnesses: &[&Randomness]) -> String {
        let mut b = String::from("[");
        for (i, r) in randomnesses.iter().enumerate() {
            if i > 0 {
                b.push(':');
            }
            b.push_str(&SeedUtils::format_seed(r.get_seed()));
        }
        b.push(']');
        b
    }
}
