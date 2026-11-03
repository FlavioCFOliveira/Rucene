//! Port of `generators/AsciiAlphanumGenerator.java` (randomizedtesting 2.8.1).

use std::ops::Deref;

use super::{CodepointSetGenerator, StringGenerator};
use crate::jdk::random::Random;

const CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// ASCII letters and digits: `AsciiAlphanumGenerator extends
/// CodepointSetGenerator`.
#[derive(Debug, Clone)]
pub struct AsciiAlphanumGenerator(CodepointSetGenerator);

impl AsciiAlphanumGenerator {
    /// `new AsciiAlphanumGenerator()`.
    pub fn new() -> AsciiAlphanumGenerator {
        let chars: Vec<u16> = CHARS.encode_utf16().collect();
        AsciiAlphanumGenerator(CodepointSetGenerator::from_chars(&chars))
    }
}

impl Default for AsciiAlphanumGenerator {
    fn default() -> Self {
        AsciiAlphanumGenerator::new()
    }
}

impl Deref for AsciiAlphanumGenerator {
    type Target = CodepointSetGenerator;

    fn deref(&self) -> &CodepointSetGenerator {
        &self.0
    }
}

impl StringGenerator for AsciiAlphanumGenerator {
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
