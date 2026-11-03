//! Port of `generators/ASCIIGenerator.java` (randomizedtesting 2.8.1).

use std::ops::Deref;

use super::{AsciiLettersGenerator, StringGenerator};
use crate::jdk::random::Random;

/// ASCII letters: `ASCIIGenerator extends AsciiLettersGenerator`
/// (deprecated in favour of [`AsciiLettersGenerator`]).
#[deprecated(note = "use AsciiLettersGenerator")]
#[derive(Debug, Clone, Default)]
pub struct AsciiGenerator(AsciiLettersGenerator);

impl AsciiGenerator {
    /// `new ASCIIGenerator()`.
    pub fn new() -> AsciiGenerator {
        AsciiGenerator(AsciiLettersGenerator::new())
    }
}

impl Deref for AsciiGenerator {
    type Target = AsciiLettersGenerator;

    fn deref(&self) -> &AsciiLettersGenerator {
        &self.0
    }
}

impl StringGenerator for AsciiGenerator {
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
