//! Port of `generators/CodepointSetGenerator.java` (randomizedtesting 2.8.1).

use super::{array_length, string_from_code_points, RandomNumbers, StringGenerator};
use crate::jdk::exception::illegal_argument;
use crate::jdk::random::Random;

/// A string generator over a fixed set of code points:
/// `CodepointSetGenerator extends StringGenerator`.
#[derive(Debug, Clone)]
pub struct CodepointSetGenerator {
    bmp: Vec<i32>,
    /// Kept as Java keeps it; only `bmp` and `all` are read.
    #[allow(dead_code)]
    supplementary: Vec<i32>,
    all: Vec<i32>,
}

impl CodepointSetGenerator {
    /// `CodepointSetGenerator(char[] chars)`: Java `char`s (UTF-16 code
    /// units), all from the BMP and none part of a surrogate pair.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` for a surrogate (`Value is part of a
    /// surrogate pair: 0x<hex>`) or an empty set (`Empty set of
    /// characters?`).
    pub fn from_chars(chars: &[u16]) -> CodepointSetGenerator {
        let mut bmp = Vec::with_capacity(chars.len());
        for &c in chars {
            bmp.push(i32::from(c));
            if (0xd800..=0xdfff).contains(&c) {
                illegal_argument(format!("Value is part of a surrogate pair: 0x{c:x}"));
            }
        }
        CodepointSetGenerator::with(bmp, Vec::new())
    }

    /// `CodepointSetGenerator(String s)`: the code points of `s`, split into
    /// BMP and supplementary ones (each list in reverse order of appearance,
    /// as Java fills them).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` (`Empty set of characters?`) for an empty
    /// string.
    pub fn from_string(s: &str) -> CodepointSetGenerator {
        let mut bmp = Vec::new();
        let mut supplementary = Vec::new();
        for c in s.chars() {
            let cp = c as i32;
            if cp >= 0x10000 {
                supplementary.push(cp);
            } else {
                bmp.push(cp);
            }
        }
        bmp.reverse();
        supplementary.reverse();
        CodepointSetGenerator::with(bmp, supplementary)
    }

    fn with(bmp: Vec<i32>, supplementary: Vec<i32>) -> CodepointSetGenerator {
        let all: Vec<i32> = bmp.iter().chain(supplementary.iter()).copied().collect();
        if all.is_empty() {
            illegal_argument("Empty set of characters?");
        }
        CodepointSetGenerator {
            bmp,
            supplementary,
            all,
        }
    }

    /// Java array lengths are `int`s.
    fn len(v: &[i32]) -> i32 {
        v.len() as i32
    }
}

impl StringGenerator for CodepointSetGenerator {
    fn of_code_units_length(
        &self,
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        let mut length = RandomNumbers::random_int_between(r, min_code_units, max_code_units);

        // With surrogate pairs only, an odd length moves to an even one.
        if self.bmp.is_empty() && length & 1 != 0 {
            if min_code_units == max_code_units {
                illegal_argument(
                    "Cannot return an odd number of code units  when surrogate pairs are the \
                     only available codepoints.",
                );
            } else if length == min_code_units {
                length += 1;
            } else {
                length -= 1;
            }
        }

        let mut codepoints = Vec::with_capacity(array_length(length));
        while length > 0 {
            let cp = if length == 1 {
                self.bmp[r.next_int_bound(Self::len(&self.bmp)) as usize]
            } else {
                self.all[r.next_int_bound(Self::len(&self.all)) as usize]
            };
            length -= if cp >= 0x10000 { 2 } else { 1 };
            codepoints.push(cp);
        }
        string_from_code_points(&codepoints)
    }

    fn of_code_points_length(
        &self,
        r: &dyn Random,
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        let length = RandomNumbers::random_int_between(r, min_code_points, max_code_points);
        let mut codepoints = vec![0i32; array_length(length)];
        // Filled from the end, as Java's `codepoints[--length]`.
        for slot in codepoints.iter_mut().rev() {
            *slot = self.all[r.next_int_bound(Self::len(&self.all)) as usize];
        }
        string_from_code_points(&codepoints)
    }
}
