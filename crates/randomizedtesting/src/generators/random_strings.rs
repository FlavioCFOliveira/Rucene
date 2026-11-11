//! Port of `com.carrotsearch.randomizedtesting.generators.RandomStrings` (randomizedtesting 2.8.1).

use std::sync::LazyLock;

#[allow(deprecated)]
use super::AsciiGenerator;
use super::{
    AsciiAlphanumGenerator, AsciiLettersGenerator, RealisticUnicodeGenerator, StringGenerator,
    UnicodeGenerator,
};
use crate::jdk::random::Random;

/// A facade over the [`StringGenerator`] implementations: `RandomStrings`.
pub struct RandomStrings;

/// `RandomStrings.realisticUnicodeGenerator`.
pub static REALISTIC_UNICODE_GENERATOR: RealisticUnicodeGenerator = RealisticUnicodeGenerator;
/// `RandomStrings.unicodeGenerator`.
pub static UNICODE_GENERATOR: UnicodeGenerator = UnicodeGenerator;
/// `RandomStrings.asciiLettersGenerator`.
pub static ASCII_LETTERS_GENERATOR: LazyLock<AsciiLettersGenerator> =
    LazyLock::new(AsciiLettersGenerator::new);
/// `RandomStrings.asciiAlphanumGenerator`.
pub static ASCII_ALPHANUM_GENERATOR: LazyLock<AsciiAlphanumGenerator> =
    LazyLock::new(AsciiAlphanumGenerator::new);
/// `RandomStrings.asciiGenerator` (deprecated).
#[allow(deprecated)]
pub static ASCII_GENERATOR: LazyLock<AsciiGenerator> = LazyLock::new(AsciiGenerator::new);

impl RandomStrings {
    /// `randomAsciiOfLengthBetween` (deprecated).
    #[deprecated(note = "use random_ascii_letters_of_length_between")]
    pub fn random_ascii_of_length_between(
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        ASCII_GENERATOR.of_code_units_length(r, min_code_units, max_code_units)
    }

    /// `randomAsciiOfLength` (deprecated).
    #[deprecated(note = "use random_ascii_letters_of_length")]
    pub fn random_ascii_of_length(r: &dyn Random, code_units: i32) -> String {
        ASCII_GENERATOR.of_code_units_length(r, code_units, code_units)
    }

    /// `randomAsciiLettersOfLengthBetween`.
    pub fn random_ascii_letters_of_length_between(
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        ASCII_LETTERS_GENERATOR.of_code_units_length(r, min_code_units, max_code_units)
    }

    /// `randomAsciiLettersOfLength`.
    pub fn random_ascii_letters_of_length(r: &dyn Random, code_units: i32) -> String {
        ASCII_LETTERS_GENERATOR.of_code_units_length(r, code_units, code_units)
    }

    /// `randomAsciiAlphanumOfLengthBetween`.
    pub fn random_ascii_alphanum_of_length_between(
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        ASCII_ALPHANUM_GENERATOR.of_code_units_length(r, min_code_units, max_code_units)
    }

    /// `randomAsciiAlphanumOfLength`.
    pub fn random_ascii_alphanum_of_length(r: &dyn Random, code_units: i32) -> String {
        ASCII_ALPHANUM_GENERATOR.of_code_units_length(r, code_units, code_units)
    }

    /// `randomUnicodeOfLengthBetween`.
    pub fn random_unicode_of_length_between(
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        UNICODE_GENERATOR.of_code_units_length(r, min_code_units, max_code_units)
    }

    /// `randomUnicodeOfLength`.
    pub fn random_unicode_of_length(r: &dyn Random, code_units: i32) -> String {
        UNICODE_GENERATOR.of_code_units_length(r, code_units, code_units)
    }

    /// `randomUnicodeOfCodepointLengthBetween`.
    pub fn random_unicode_of_codepoint_length_between(
        r: &dyn Random,
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        UNICODE_GENERATOR.of_code_points_length(r, min_code_points, max_code_points)
    }

    /// `randomUnicodeOfCodepointLength`.
    pub fn random_unicode_of_codepoint_length(r: &dyn Random, code_points: i32) -> String {
        UNICODE_GENERATOR.of_code_points_length(r, code_points, code_points)
    }

    /// `randomRealisticUnicodeOfLengthBetween`.
    pub fn random_realistic_unicode_of_length_between(
        r: &dyn Random,
        min_code_units: i32,
        max_code_units: i32,
    ) -> String {
        REALISTIC_UNICODE_GENERATOR.of_code_units_length(r, min_code_units, max_code_units)
    }

    /// `randomRealisticUnicodeOfLength`.
    pub fn random_realistic_unicode_of_length(r: &dyn Random, code_units: i32) -> String {
        REALISTIC_UNICODE_GENERATOR.of_code_units_length(r, code_units, code_units)
    }

    /// `randomRealisticUnicodeOfCodepointLengthBetween`.
    pub fn random_realistic_unicode_of_codepoint_length_between(
        r: &dyn Random,
        min_code_points: i32,
        max_code_points: i32,
    ) -> String {
        REALISTIC_UNICODE_GENERATOR.of_code_points_length(r, min_code_points, max_code_points)
    }

    /// `randomRealisticUnicodeOfCodepointLength`.
    pub fn random_realistic_unicode_of_codepoint_length(
        r: &dyn Random,
        code_points: i32,
    ) -> String {
        REALISTIC_UNICODE_GENERATOR.of_code_points_length(r, code_points, code_points)
    }
}
