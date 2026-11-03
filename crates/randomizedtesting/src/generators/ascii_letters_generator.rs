//! Port of `com.carrotsearch.randomizedtesting.generators.AsciiLettersGenerator` (randomizedtesting 2.8.1).

use std::ops::Deref;

use super::{CodepointSetGenerator, StringGenerator};
use crate::jdk::random::Random;

const CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";

/// ASCII letters, `a-z` and `A-Z`: `AsciiLettersGenerator extends
/// CodepointSetGenerator`.
#[derive(Debug, Clone)]
pub struct AsciiLettersGenerator(CodepointSetGenerator);

impl AsciiLettersGenerator {
    /// `new AsciiLettersGenerator()`.
    pub fn new() -> AsciiLettersGenerator {
        let chars: Vec<u16> = CHARS.encode_utf16().collect();
        AsciiLettersGenerator(CodepointSetGenerator::from_chars(&chars))
    }
}

impl Default for AsciiLettersGenerator {
    fn default() -> Self {
        AsciiLettersGenerator::new()
    }
}

impl Deref for AsciiLettersGenerator {
    type Target = CodepointSetGenerator;

    fn deref(&self) -> &CodepointSetGenerator {
        &self.0
    }
}

impl StringGenerator for AsciiLettersGenerator {
    fn of_code_units_length(
        &self,
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        self.0
            .of_code_units_length(r, min_code_units, max_code_units)
    }

    fn of_code_points_length(
        &self,
        r: &dyn Random,
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        self.0
            .of_code_points_length(r, min_code_points, max_code_points)
    }
}
